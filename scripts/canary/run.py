#!/usr/bin/env python3
"""Process-group runner (nested spec §D2): `run.py --timeout S [--until-file F] -- argv...`.

Starts argv in its own session/process group and kills the whole group when the deadline passes, when F
appears, or when the leader exits (no orphan survives). Exit code: the leader's own code when it exits first;
0 when F appeared; 124 on timeout; 127 when argv cannot be started. Pure stdlib."""
import argparse, os, signal, subprocess, sys, time


def _kill_group(proc):
    """SIGTERM then SIGKILL the leader's group. macOS answers EPERM for a group whose only member is an
    unreaped zombie, so the leader is reaped between the two signals and EPERM counts as 'gone'."""
    for sig in (signal.SIGTERM, signal.SIGKILL):
        try:
            os.killpg(proc.pid, sig)
        except (ProcessLookupError, PermissionError):
            break
        try:
            proc.wait(timeout=0.5)
        except subprocess.TimeoutExpired:
            pass
    proc.wait()


def run(argv, timeout, until_file=None, poll=0.05):
    try:
        proc = subprocess.Popen(argv, start_new_session=True)
    except OSError as e:
        print(f"run.py: cannot start {argv[0]}: {e}", file=sys.stderr)
        return 127
    deadline = time.monotonic() + timeout
    code = None
    try:
        while True:
            rc = proc.poll()
            if rc is not None:
                code = rc if rc >= 0 else 128 - rc
                break
            if until_file and os.path.exists(until_file):
                code = 0
                break
            if time.monotonic() >= deadline:
                code = 124
                break
            time.sleep(poll)
    finally:
        _kill_group(proc)
    return code


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--timeout", type=float, required=True)
    ap.add_argument("--until-file")
    ap.add_argument("argv", nargs=argparse.REMAINDER)
    args = ap.parse_args(argv)
    cmd = args.argv[1:] if args.argv[:1] == ["--"] else args.argv
    if not cmd:
        ap.error("no command given")
    return run(cmd, args.timeout, args.until_file)



# Strategy execution shares the bounded process ownership above with legacy probes.
import hashlib
import importlib.util
import json
import pathlib
import re
import selectors

HERE = pathlib.Path(__file__).resolve().parent


def _sibling(name):
    key = "canary_" + name
    if key not in sys.modules:
        spec = importlib.util.spec_from_file_location(key, HERE / (name + ".py"))
        mod = importlib.util.module_from_spec(spec)
        sys.modules[key] = mod
        spec.loader.exec_module(mod)
    return sys.modules[key]


def bounded_capture(argv, timeout=60, env=None, cwd=None, stdout_limit=65536, stderr_limit=8192):
    """Read bounded pipes while enforcing one deadline; always cancel/reap the owned group."""
    proc = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            stdin=subprocess.DEVNULL, start_new_session=True, env=env, cwd=cwd)
    chunks = [bytearray(), bytearray()]
    deadline = time.monotonic() + timeout
    try:
        with selectors.DefaultSelector() as poller:
            poller.register(proc.stdout, selectors.EVENT_READ, 0)
            poller.register(proc.stderr, selectors.EVENT_READ, 1)
            while poller.get_map():
                left = deadline - time.monotonic()
                if left <= 0:
                    raise ValueError("companion deadline exceeded")
                for key, _ in poller.select(min(left, .05)):
                    data = os.read(key.fileobj.fileno(), 4096)
                    if not data:
                        poller.unregister(key.fileobj)
                        continue
                    n = key.data
                    chunks[n].extend(data)
                    if len(chunks[n]) > (stdout_limit, stderr_limit)[n]:
                        raise ValueError(("stdout", "stderr")[n] + " limit exceeded")
            try:
                code = proc.wait(timeout=max(.001, deadline - time.monotonic()))
            except subprocess.TimeoutExpired as e:
                raise ValueError("companion deadline exceeded") from e
        return code, bytes(chunks[0]), bytes(chunks[1])
    finally:
        _kill_group(proc)
        proc.stdout.close()
        proc.stderr.close()


def _pairs(pairs):
    out = {}
    for key, value in pairs:
        if key in out:
            raise ValueError("duplicate JSON field")
        out[key] = value
    return out


def _read_bounded(path, cap):
    with open(path, "rb") as source:
        raw = source.read(cap + 1)
    if len(raw) > cap:
        raise ValueError("artifact byte limit exceeded")
    return raw


def _json(raw, cap):
    if len(raw) > cap:
        raise ValueError("JSON byte limit exceeded")
    try:
        return json.loads(raw.decode("utf-8"), object_pairs_hook=_pairs,
                          parse_constant=lambda x: (_ for _ in ()).throw(ValueError("invalid JSON number")))
    except (UnicodeError, json.JSONDecodeError) as e:
        raise ValueError("invalid UTF-8 JSON") from e


def _schema(value, spec):
    """Validate the frozen companion/index schemas without a third-party dependency."""
    if "anyOf" in spec:
        for option in spec["anyOf"]:
            try:
                _schema(value, option)
                break
            except ValueError:
                pass
        else:
            raise ValueError("no schema alternative matches")
    if "const" in spec and (type(value) is not type(spec["const"]) or value != spec["const"]):
        raise ValueError("invalid constant")
    if "enum" in spec and not any(type(value) is type(x) and value == x for x in spec["enum"]):
        raise ValueError("invalid enum")
    types = {"object": dict, "array": list, "string": str, "integer": int, "boolean": bool, "null": type(None)}
    kind = spec.get("type")
    if kind and type(value) is not types[kind]:
        raise ValueError("invalid type")
    if isinstance(value, dict):
        if any(k not in value for k in spec.get("required", [])):
            raise ValueError("missing required field")
        props = spec.get("properties", {})
        if spec.get("additionalProperties") is False and set(value) - set(props):
            raise ValueError("unknown field")
        for k, v in value.items():
            if k in props:
                _schema(v, props[k])
    if isinstance(value, list):
        if not spec.get("minItems", 0) <= len(value) <= spec.get("maxItems", len(value)):
            raise ValueError("array bound exceeded")
        if spec.get("uniqueItems") and len({json.dumps(x, sort_keys=True) for x in value}) != len(value):
            raise ValueError("duplicate array item")
        for x in value:
            if "items" in spec:
                _schema(x, spec["items"])
    if isinstance(value, str):
        if not spec.get("minLength", 0) <= len(value.encode()) <= spec.get("maxLength", len(value.encode())):
            raise ValueError("string byte bound exceeded")
        if "pattern" in spec and not re.search(spec["pattern"], value):
            raise ValueError("invalid string")
    if type(value) is int and not spec.get("minimum", value) <= value <= spec.get("maximum", value):
        raise ValueError("integer out of bounds")
    for rule in spec.get("allOf", []):
        try:
            _schema(value, rule["if"])
        except ValueError:
            continue
        _schema(value, rule["then"])


def validate_identity(identity):
    if identity is None:
        return
    descriptor = {k: v for k, v in identity.items() if k != "key"}
    if identity["key"].startswith("release:"):
        version = identity["release_version"]
        stable = version is not None and _sibling("versions").STABLE.fullmatch(version)
        if (not stable or identity["key"] != "release:" + version or identity["dirty"] is True
                or identity["distance"] not in (None, 0) or identity["commit"] is not None
                or identity["base_version"] not in (None, version)
                or identity["derived_version"] not in (None, version)):
            raise ValueError("release identity mismatch")
    else:
        canonical = json.dumps(dict(descriptor, schema_version=1), sort_keys=True,
                               separators=(",", ":"), ensure_ascii=False).encode()
        if identity["key"] != "build:" + hashlib.sha256(canonical).hexdigest():
            raise ValueError("build identity hash mismatch")


def validate_result(raw, adapter, attempt, stage):
    doc = _json(raw, 65536)
    _schema(doc, json.loads((HERE / "companion_schema.json").read_text()))
    if (doc["harness"], doc["attempt"], doc["evidence_stage"]) != (adapter["id"], attempt, stage):
        raise ValueError("attempt identity/stage mismatch")
    validate_identity(doc["identity"])
    contracts = {c["domain"]: c for c in adapter["contracts"]}
    seen = set()
    if len(doc["domains"]) > len(contracts):
        raise ValueError("domain count exceeded")
    for d in doc["domains"]:
        c = contracts.get(d["domain"])
        if not c or d["domain"] in seen or (d["origin"], d["contract_id"]) != (c["origin"], c["id"]):
            raise ValueError("undeclared/duplicate domain")
        seen.add(d["domain"])
        milestones = {e["milestone"] for e in c["events"] if e["milestone"] is not None}
        if set(d["successful_milestones"]) - milestones:
            raise ValueError("undeclared milestone")
        events = {e["event"] for e in c["events"]}
        if any(v["event"] not in events for v in d["violations"]):
            raise ValueError("undeclared violation event")
    if doc["outcome"] == "complete" and (seen != set(contracts)
            or any(d["outcome"] == "inconclusive" for d in doc["domains"])):
        raise ValueError("inconsistent complete outcome")
    return doc


def verified_domains(result, adapter):
    if result["identity"] is None or result["outcome"] != "complete" or result["evidence_stage"] == "source_captured":
        return []
    validate_identity(result["identity"])
    contracts = {c["domain"]: c for c in adapter["contracts"]}
    return [d for d in result["domains"] if d["outcome"] == "compatible"
            and set(contracts[d["domain"]]["required_milestones"]) <= set(d["successful_milestones"])]


def _relative(root, path):
    if (not isinstance(path, str) or len(path.encode()) > 256
            or not re.fullmatch(r"[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)*", path)
            or any(p in (".", "..") for p in path.split("/"))):
        raise ValueError("invalid relative path")
    candidate = pathlib.Path(root) / path
    if not candidate.resolve().is_relative_to(pathlib.Path(root).resolve()):
        raise ValueError("symlink path escape")
    return candidate


def validate_index(raw, root, adapters):
    doc = _json(raw, 262144)
    _schema(doc, json.loads((HERE / "artifact_index_schema.json").read_text()))
    registry = {a["id"]: a for a in adapters}
    seen = set()
    for entry in doc["attempts"]:
        pair = entry["harness"], entry["attempt"]
        if pair in seen or pair[0] not in registry:
            raise ValueError("unknown/duplicate indexed attempt")
        seen.add(pair)
        attempt_dir = pathlib.Path(root) / "work" / ("-".join(pair))
        path = _relative(root, entry["result_path"])
        for capture in [path] + [_relative(root, p) for p in entry["capture_paths"]]:
            if not capture.resolve().is_relative_to(attempt_dir.resolve()) or not capture.is_file():
                raise ValueError("artifact outside attempt directory")
        result = validate_result(_read_bounded(path, 65536), registry[pair[0]], pair[1], entry["evidence_stage"])
        identity = result["identity"]
        if entry["identity_key"] != (identity["key"] if identity else None):
            raise ValueError("index identity mismatch")
        for capture in entry["capture_paths"]:
            cap = _json(_read_bounded(_relative(root, capture), 65536), 65536)
            if set(cap) != {"domain", "origin", "evidence_stage", "path"}:
                raise ValueError("invalid capture metadata")
            if cap["evidence_stage"] != entry["evidence_stage"] or not any(
                    (d["domain"], d["origin"]) == (cap["domain"], cap["origin"]) for d in result["domains"]):
                raise ValueError("capture domain/stage mismatch")
            target = _relative(attempt_dir, cap["path"])
            if not target.is_file():
                raise ValueError("missing indexed capture")
    return doc


def _strict(value, keys):
    if not isinstance(value, dict) or set(value) != set(keys.split()):
        raise ValueError("invalid discovery fields")


def _text(value, pattern, cap):
    if not isinstance(value, str) or not len(value.encode()) <= cap or not re.fullmatch(pattern, value):
        raise ValueError("invalid discovery token")


def _unique(values, cap):
    if not isinstance(values, list) or len(values) > cap or len({json.dumps(v, sort_keys=True) for v in values}) != len(values):
        raise ValueError("invalid discovery collection")


def validate_discovery(discovery):
    _strict(discovery, "schema_version adapters")
    if type(discovery["schema_version"]) is not int or discovery["schema_version"] != 1:
        raise ValueError("unsupported discovery schema")
    _unique(discovery["adapters"], 64)
    ids = set()
    for a in discovery["adapters"]:
        _strict(a, "id display_name host_kinds setup_scopes legacy_contract_id contracts canary_strategy")
        _text(a["id"], r"[a-z][a-z0-9_-]*", 64)
        if a["id"] in ids:
            raise ValueError("duplicate adapter id")
        ids.add(a["id"])
        _text(a["display_name"], r"[^\x00-\x1f\x7f]+", 128)
        _unique(a["host_kinds"], 8)
        for host in a["host_kinds"]:
            _text(host, r"[a-z][a-z0-9_-]*", 64)
        _unique(a["setup_scopes"], 2)
        if any(scope not in ("config_root", "profile") for scope in a["setup_scopes"]):
            raise ValueError("invalid setup scope")
        if a["legacy_contract_id"] is not None:
            _text(a["legacy_contract_id"], "[a-f0-9]{16}", 16)
        _unique(a["contracts"], 8)
        domains = set()
        for c in a["contracts"]:
            _strict(c, "domain origin id events required_milestones")
            _text(c["domain"], "[a-z][a-z0-9_]*", 32)
            _text(c["id"], "[a-f0-9]{16}", 16)
            if c["domain"] in domains or c["origin"] not in ("native_payload", "native_shape_observation", "bridge_envelope"):
                raise ValueError("duplicate/invalid discovery domain")
            domains.add(c["domain"])
            _unique(c["events"], 32)
            _unique(c["required_milestones"], 8)
            event_names, milestones = set(), set()
            for event in c["events"]:
                _strict(event, "event milestone always_send")
                _text(event["event"], "[A-Za-z][A-Za-z0-9_]*", 63)
                if event["event"] in event_names or type(event["always_send"]) is not bool:
                    raise ValueError("invalid discovery event")
                event_names.add(event["event"])
                if event["milestone"] is not None:
                    _text(event["milestone"], "[a-z][a-z0-9_]*", 32)
                    milestones.add(event["milestone"])
            for milestone in c["required_milestones"]:
                _text(milestone, "[a-z][a-z0-9_]*", 32)
                if milestone not in milestones:
                    raise ValueError("undeclared required milestone")
        strategy = a["canary_strategy"]
        if strategy is not None:
            _strict(strategy, "kind candidate_kind npm_package model_key_env companion artifact_schema_version")
            companion_path(HERE.parents[1], strategy)
            if type(strategy["artifact_schema_version"]) is not int:
                raise ValueError("invalid artifact schema type")
            if strategy["model_key_env"] is not None:
                _text(strategy["model_key_env"], "[A-Z][A-Z0-9_]*", 64)
            if (strategy["kind"], strategy["candidate_kind"]) == ("npm_release", "stable_release"):
                _text(strategy["npm_package"], r"(?:@[a-z0-9_.-]+/)?[a-z0-9_.-]+", 128)
            elif (strategy["kind"], strategy["candidate_kind"], strategy["npm_package"]) != ("exact_runtime", "exact_build", None):
                raise ValueError("invalid strategy kind")
    if len(json.dumps(discovery).encode()) > 65536:
        raise ValueError("discovery byte limit exceeded")


def select_adapters(discovery, selector):
    validate_discovery(discovery)
    adapters = discovery["adapters"]
    ids = [a["id"] for a in adapters]
    if len(adapters) > 64 or len(ids) != len(set(ids)) or any(
            not re.fullmatch(r"[a-z][a-z0-9_-]{0,63}", x) for x in ids):
        raise ValueError("invalid adapter registry")
    if selector == "all":
        return adapters
    chosen = ["claude", "codex"] if selector == "both" else [selector]
    if any(x not in ids for x in chosen):
        raise ValueError("unknown harness")
    return [a for a in adapters if a["id"] in chosen]


def isolated_env(work, strategy, stage):
    work = pathlib.Path(work)
    env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "LANG": "C.UTF-8", "TERM": "dumb",
           "PYTHONDONTWRITEBYTECODE": "1"}
    for key, rel in [("HOME", "home"), ("CLAUDE_CONFIG_DIR", "home/.claude"), ("CODEX_HOME", "home/.codex"),
                     ("XDG_CONFIG_HOME", "home/.config"), ("XDG_STATE_HOME", "home/.local/state"),
                     ("XDG_DATA_HOME", "home/.local/share"), ("XDG_CACHE_HOME", "home/.cache"), ("TMPDIR", "tmp")]:
        path = work / rel
        path.mkdir(parents=True, exist_ok=True)
        env[key] = str(path)
    caller_home = pathlib.Path(os.path.expanduser("~"))
    for key, rel in (("CARGO_HOME", ".cargo"), ("RUSTUP_HOME", ".rustup")):
        env[key] = os.environ.get(key) or str(caller_home / rel)
    for key in ("RUSTUP_TOOLCHAIN", "CARGO_TARGET_DIR"):
        if key in os.environ:
            env[key] = os.environ[key]
    key = strategy.get("model_key_env")
    if stage == "live" and key and os.environ.get(key):
        env[key] = os.environ[key]
    return env


def inconclusive(adapter, reason, stage="no_model"):
    return {"harness": adapter["id"], "package": (adapter.get("canary_strategy") or {}).get("npm_package"),
            "candidate_kind": (adapter.get("canary_strategy") or {}).get("candidate_kind"),
            "verified_max": None, "candidates": [], "status": "inconclusive", "first_bad": None,
            "last_good": None, "failing_checks": [], "signals": [], "suggested_action": None,
            "excluded": [], "probes": [], "assumes_monotone": False, "reason": reason,
            "evidence_stage": stage}


def companion_path(root, strategy):
    if strategy.get("artifact_schema_version") != 1:
        raise ValueError("unsupported companion schema")
    path = strategy.get("companion")
    if not isinstance(path, str) or not path.startswith("scripts/canary/adapters/"):
        raise ValueError("invalid companion root")
    return _relative(root, path)


def invoke(adapter, root, binary, out, token, stage, request, index, runtime_command=None, timeout=1800):
    strategy = adapter["canary_strategy"]
    work = pathlib.Path(out) / "work" / (adapter["id"] + "-" + token)
    work.mkdir(parents=True)
    (work / "request.json").write_text(json.dumps(dict(request, adapter=adapter)))
    argv = [sys.executable, str(companion_path(root, strategy)), "--harness", adapter["id"],
            "--attempt", token, "--stage", stage, "--work-dir", str(work), "--binary", str(binary)]
    if runtime_command:
        argv += ["--runtime-command-file", str(pathlib.Path(runtime_command).resolve())]
    isolation = _sibling("isolation")
    watched = isolation.watched_paths()
    before = isolation.snapshot(watched)
    try:
        rc, raw, err = bounded_capture(argv, timeout=timeout, cwd=work, env=isolated_env(work, strategy, stage))
    finally:
        untouched, _ = isolation.compare(before, isolation.snapshot(watched))
        if not untouched:
            raise ValueError("caller config isolation tripwire changed")
    if rc != 0:
        raise ValueError("companion infrastructure failure")
    result = validate_result(raw, adapter, token, stage)
    path = work / "result.json"
    path.write_bytes(raw)
    diagnostic = err.decode("utf-8", "replace")
    key = strategy.get("model_key_env")
    if key and os.environ.get(key):
        diagnostic = diagnostic.replace(os.environ[key], "[redacted]")
    sanitized = "".join(c for c in diagnostic if c.isprintable()).encode("utf-8")
    (work / "stderr.txt").write_bytes(sanitized[:8192].decode("utf-8", "ignore").encode("utf-8"))
    captures = []
    capture_index = work / "captures.json"
    if capture_index.exists():
        captures = _json(_read_bounded(capture_index, 65536), 65536)
        if not isinstance(captures, list) or len(captures) > 8:
            raise ValueError("invalid companion capture index")
        captures = [str(_relative(work, p).relative_to(out)) for p in captures]
    entry = {"harness": adapter["id"], "attempt": token,
             "identity_key": result["identity"]["key"] if result["identity"] else None,
             "evidence_stage": stage, "result_path": str(path.relative_to(out)), "capture_paths": captures}
    proposed = dict(index, attempts=index["attempts"] + [entry])
    raw_index = json.dumps(proposed).encode()
    _json(raw_index, 262144)
    _schema(proposed, json.loads((HERE / "artifact_index_schema.json").read_text()))
    validate_index(json.dumps(dict(index, attempts=[entry])).encode(), out, [adapter])
    index["attempts"].append(entry)
    return result, work


def run_strategy(adapter, out, binary, root, model_tier="off", runtime_command=None,
                 versions_mode="since-verified", baseline_doc=None, baseline=None, bisect=False,
                 index=None, recipes=None, evidence_stage=None):
    strategy = adapter.get("canary_strategy")
    stage = "live" if model_tier != "off" and strategy and strategy.get("model_key_env") and os.environ.get(
        strategy["model_key_env"]) else "no_model"
    if evidence_stage:
        stage = evidence_stage
    block = inconclusive(adapter, "unsupported strategy", stage)
    if stage == "live" and (model_tier == "off" or not strategy or not strategy.get("model_key_env")
                            or not os.environ.get(strategy["model_key_env"])):
        return dict(block, reason="live stage requires an enabled model tier and declared credential")
    if strategy is None:
        return block
    try:
        companion = companion_path(root, strategy)
        if not companion.is_file():
            return dict(block, reason="missing companion")
        if model_tier == "required" and (not strategy.get("model_key_env") or stage != "live"):
            return dict(block, reason="missing declared model key")
        if strategy["kind"] == "exact_runtime":
            if strategy["candidate_kind"] != "exact_build" or strategy.get("npm_package") is not None:
                raise ValueError("invalid exact-runtime strategy")
            if not runtime_command or not pathlib.Path(runtime_command).is_file():
                return dict(block, reason="explicit runtime command input required")
            local_index = index if index is not None else {"schema_version": 1, "attempts": []}
            token = "attempt-" + str(len(local_index["attempts"]) + 1)
            result, _ = invoke(adapter, root, binary, out, token, stage, {}, local_index, runtime_command)
            block["identity"] = result["identity"]
            block["domains"] = result["domains"]
            block["reason"] = result["reason"]
            if result["outcome"] == "complete" and len(verified_domains(result, adapter)) == len(adapter["contracts"]) and adapter["contracts"]:
                block["status"] = "all_pass"
            elif result["outcome"] == "complete" and result["identity"] is not None and stage != "source_captured" and any(
                    d["outcome"] == "contract_violation" for d in result["domains"]):
                block["status"] = "break"
            return block
        if strategy["kind"] != "npm_release" or strategy["candidate_kind"] != "stable_release" or not strategy.get("npm_package"):
            raise ValueError("invalid stable-release strategy")
        view_dir = pathlib.Path(out) / "work" / ("view-" + adapter["id"])
        env = isolated_env(view_dir, strategy, "no_model")
        env["npm_config_cache"] = str(view_dir / "npm-cache")
        env["npm_config_update_notifier"] = "false"
        for _ in range(3):
            try:
                rc, raw, _ = bounded_capture(["npm", "view", strategy["npm_package"], "versions", "--json"],
                                             timeout=60, env=env)
                if rc == 0:
                    published = _json(raw, 65536)
                    break
            except (ValueError, OSError):
                pass
        else:
            return dict(block, status="infra_error", reason="npm view failed after three attempts")
        if isinstance(published, str):
            published = [published]
        versions = _sibling("versions")
        doc = baseline_doc or {"rows": []}
        contract = adapter["legacy_contract_id"]
        explicit = versions_mode.split(",") if versions_mode not in versions.MODES else None
        cands = versions.candidates(published, versions_mode, doc, adapter["id"], explicit, contract)
        vm = versions.verified_max(doc, adapter["id"], contract)
        local_index = index if index is not None else {"schema_version": 1, "attempts": []}
        attempt_count = [0]
        incomplete_runtime = [False]
        def probe(version, tier1):
            attempt_count[0] += 1
            token = "attempt-" + str(attempt_count[0])
            start = time.monotonic()
            try:
                result, work = invoke(adapter, root, binary, out, token, stage,
                                      {"version": version, "recipes": recipes, "model_tier": model_tier},
                                      local_index)
                legacy = work / "legacy-probe.json"
                if result["identity"] is not None and result["identity"]["key"] != "release:" + version:
                    raise ValueError("npm attempt release identity mismatch")
                if legacy.is_file():
                    parsed = _sibling("bisect").parse_probe_output(_read_bounded(legacy, 65536).decode("utf-8"))
                    verdict = parsed["result"]
                    checks, ft = parsed["checks"], parsed["failed_tier"]
                    contract_result = parsed.get("contract")
                    # Historical semver probes remain independent of rich milestone verification.
                else:
                    verdict = "pass" if result["outcome"] == "complete" and (
                        len(verified_domains(result, adapter)) == len(adapter["contracts"])) and adapter["contracts"] else "infra"
                    if result["outcome"] == "complete" and any(
                            d["outcome"] == "contract_violation" for d in result["domains"]):
                        verdict = "fail"
                    checks, ft, contract_result = [], None, None
                if result["outcome"] == "infra_failure":
                    verdict = "infra"
                if (result["identity"] is None or result["outcome"] == "unsupported" or
                        (not legacy.is_file() and result["outcome"] == "inconclusive") or stage == "source_captured"):
                    verdict = "infra"
                    incomplete_runtime[0] = True
                att = {"result": verdict, "tier1": stage == "live", "duration_ms": int((time.monotonic()-start)*1000),
                       "checks": checks}
                if contract_result:
                    att["contract"] = contract_result
                return att, ft
            except (ValueError, OSError) as e:
                return {"result": "infra", "tier1": stage == "live", "duration_ms": 0,
                        "checks": [{"id": "companion", "status": "fail", "detail": str(e)}]}, None
        search = _sibling("bisect").run(cands, baseline or vm or "0.0.0", probe, bisect=bisect,
                tier1=stage == "live", known_broken=versions.known_broken(doc, adapter["id"], contract),
                reprobe=versions.reprobe(published, doc, adapter["id"], contract) if versions_mode == "since-verified" else [])
        result_block = _sibling("report").harness_block(adapter["id"], search, vm, strategy["npm_package"])
        result_block.update(candidate_kind="stable_release", evidence_stage=stage, verified_max=vm)
        if search["status"] == "infra_error" and incomplete_runtime[0]:
            result_block.update(status="inconclusive", reason="runtime attribution or requested-stage evidence incomplete")
        return result_block
    except (ValueError, OSError, KeyError) as e:
        return dict(block, reason=str(e))


def normalize_legacy(probe, adapter, attempt, stage, version):
    """Normalize only parsed native captures; metadata/completed checks never invent milestones."""
    identity = None
    if any(c["id"] == "t0.version" and c["status"] == "pass" for c in probe["checks"]):
        identity = {"key": "release:" + version, "release_version": version, "source": "npm",
                    "base_version": None, "derived_version": None, "commit": None, "dirty": None, "distance": None}
    payloads = (probe.get("contract") or {}).get("payloads", [])
    domains = []
    for contract in adapter["contracts"]:
        successful, violations = [], []
        # Legacy captures are native payloads, never bridge/native-shape evidence.
        if contract["origin"] == "native_payload":
            for payload in payloads:
                event = next((e for e in contract["events"] if e["event"] == payload["event"]), None)
                if event is None:
                    continue
                if payload["kind"] == "ok" and event["milestone"] and event["milestone"] not in successful:
                    successful.append(event["milestone"])
                if payload["kind"] == "violation" and payload.get("field"):
                    v = {"event": event["event"], "field": payload["field"]}
                    if v not in violations:
                        violations.append(v)
        outcome = "contract_violation" if violations else "compatible" if successful else "inconclusive"
        domains.append({"domain": contract["domain"], "origin": contract["origin"], "contract_id": contract["id"],
                        "successful_milestones": successful, "violations": violations, "outcome": outcome})
    complete = bool(domains) and all(d["outcome"] != "inconclusive" for d in domains)
    return {"schema_version": 1, "harness": adapter["id"], "attempt": attempt, "identity": identity,
            "evidence_stage": stage, "outcome": "complete" if complete else "inconclusive",
            "reason": None if complete else "required native payload evidence unavailable", "domains": domains}


def index_legacy_captures(work, result, harness, version, stage):
    captures = []
    for domain in result["domains"]:
        if domain["origin"] != "native_payload":
            continue
        for tier in ("tier0", "tier1"):
            if stage == "no_model" and tier == "tier1":
                continue
            for directory in sorted((work / "legacy/work").glob(harness + "-" + version + "-*")):
                paths = sorted((directory / "capture" / tier).iterdir()) if (directory / "capture" / tier).is_dir() else []
                for capture in paths:
                    if capture.suffix not in (".stdin", ".argv", ".json") or len(captures) >= 8:
                        continue
                    relative = str(capture.relative_to(work))
                    _read_bounded(_relative(work, relative), 65536)
                    metadata = {"domain": domain["domain"], "origin": domain["origin"],
                                "evidence_stage": stage, "path": relative}
                    name = "capture-" + str(len(captures)) + ".json"
                    (work / name).write_text(json.dumps(metadata))
                    captures.append(name)
    return captures


def _interrupted(signum, frame):
    raise SystemExit(128 + signum)


def legacy_companion(argv, harness, companion):
    """Adapter-owned shell mechanics reuse common legacy checks and capture normalization."""
    signal.signal(signal.SIGTERM, _interrupted)
    signal.signal(signal.SIGINT, _interrupted)
    ap = argparse.ArgumentParser()
    ap.add_argument("--harness", required=True)
    ap.add_argument("--attempt", required=True)
    ap.add_argument("--stage", choices=("source_captured", "no_model", "live"), required=True)
    ap.add_argument("--work-dir", required=True)
    ap.add_argument("--binary", required=True)
    args = ap.parse_args(argv)
    if args.harness != harness:
        ap.error("unsupported owned companion request")
    work = pathlib.Path(args.work_dir).resolve()
    request = _json(_read_bounded(work / "request.json", 65536), 65536)
    adapter = request["adapter"]
    strategy = adapter["canary_strategy"]
    version = request["version"]
    if not _sibling("versions").STABLE.fullmatch(version):
        raise ValueError("not a stable release")
    root = HERE.parents[1]
    cmd = ["bash", str(root / "scripts/harness-canary.sh"), "--probe", harness, version,
           "--out", str(work / "legacy"), "--keep", "--herdr-threads", args.binary,
           "--model-tier", "required" if args.stage == "live" else "off"]
    if request.get("recipes"):
        cmd += ["--versions-json", request["recipes"]]
    env = dict(os.environ, HT_CANARY_COMPANION=str(companion), HT_CANARY_PACKAGE=strategy["npm_package"])
    rc, raw, _ = bounded_capture(cmd, timeout=1750, env=env)
    probe = _sibling("bisect").parse_probe_output(raw.decode("utf-8"))
    if rc != {"pass": 0, "fail": 1, "infra": 2}[probe["result"]]:
        raise ValueError("legacy probe result/exit mismatch")
    (work / "legacy-probe.json").write_bytes(raw)
    result = normalize_legacy(probe, adapter, args.attempt, args.stage, version)
    validate_result(json.dumps(result).encode(), adapter, args.attempt, args.stage)
    captures = index_legacy_captures(work, result, harness, version, args.stage)
    (work / "captures.json").write_text(json.dumps(captures))
    print(json.dumps(result))
    return 0


def strategy_main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument("--binary", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--harness", default="all")
    ap.add_argument("--model-tier", choices=("auto", "off", "required"), default="auto")
    ap.add_argument("--versions", default="since-verified")
    ap.add_argument("--baseline-json")
    ap.add_argument("--versions-json")
    ap.add_argument("--baseline")
    ap.add_argument("--bisect", action="store_true")
    ap.add_argument("--keep", action="store_true")
    ap.add_argument("--runtime-command-file")
    ap.add_argument("--evidence-stage", choices=("source_captured", "no_model", "live"))
    args = ap.parse_args(argv)
    root = HERE.parents[1]
    out = pathlib.Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    rc, raw, _ = bounded_capture([args.binary, "adapters", "--json"], timeout=30)
    if rc:
        raise ValueError("same-binary adapter discovery failed")
    registry = _json(raw, 65536)
    adapters = select_adapters(registry, args.harness)
    doc = _sibling("versions").load_versions_json(args.baseline_json) if args.baseline_json else {"rows": []}
    index = {"schema_version": 1, "attempts": []}
    blocks = [run_strategy(a, out, pathlib.Path(args.binary).resolve(), root, args.model_tier,
                          args.runtime_command_file, args.versions, doc, args.baseline, args.bisect,
                          index, args.versions_json, args.evidence_stage) for a in adapters]
    validate_index(json.dumps(index).encode(), out, adapters)
    (out / "artifact-index.json").write_text(json.dumps(index, indent=2) + "\n")
    commit = bounded_capture(["git", "-C", str(root), "rev-parse", "HEAD"], timeout=5)[1].decode().strip()
    inputs = {key: getattr(args, key) for key in ("harness", "versions", "bisect", "model_tier")}
    version = re.search(r'^version\s*=\s*"([^"]+)"', (root / "Cargo.toml").read_text(), re.MULTILINE)
    report = _sibling("report").assemble(blocks, inputs, {"os": sys.platform, "arch": os.uname().machine},
                                       commit, version.group(1) if version else "unknown")
    _sibling("report").write(report, out)
    return report["exit_code"]


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, _interrupted)
    signal.signal(signal.SIGINT, _interrupted)
    try:
        sys.exit(strategy_main(sys.argv[2:]) if sys.argv[1:2] == ["strategy"] else main())
    except (ValueError, OSError, KeyError) as e:
        print("canary: " + str(e), file=sys.stderr)
        sys.exit(2)
