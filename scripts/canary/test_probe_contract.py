import importlib.util, json, os, pathlib, re, subprocess, sys, unittest

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent.parent
SCHEMA = json.loads((HERE / "probe_schema.json").read_text())

# scripts/canary/bisect.py shadows the stdlib `bisect` when `discover -s scripts/canary` puts this
# directory first on sys.path, which breaks `random`/`tempfile`. Load the stdlib module first
# (with this directory off sys.path), then load bisect.py under another name.
_saved = list(sys.path)
sys.path[:] = [p for p in sys.path if os.path.realpath(p or ".") != str(HERE)]
sys.modules.pop("bisect", None)
import bisect as _stdlib_bisect  # noqa: E402,F401
import tempfile  # noqa: E402
sys.path[:] = _saved
_spec = importlib.util.spec_from_file_location("canary_bisect", HERE / "bisect.py")
canary_bisect = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(canary_bisect)


def validate(instance, schema, root, path=""):
    """Subset validator: type, required, properties, additionalProperties:false, enum, items, pattern, $ref."""
    if "$ref" in schema:
        node = root
        for part in schema["$ref"][2:].split("/"):
            node = node[part]
        return validate(instance, node, root, path)
    if "enum" in schema:
        ok = any(instance == e and type(instance) is type(e) for e in schema["enum"])
        if not ok:
            raise AssertionError(f"{path or '/'}: {instance!r} not in enum")
    t = schema.get("type")
    pytypes = {"object": dict, "array": list, "string": str, "null": type(None)}
    if isinstance(t, list):
        if not any(isinstance(instance, pytypes[x]) for x in t):
            raise AssertionError(f"{path or '/'}: expected one of {t}")
        t = None if instance is None or isinstance(instance, str) else t[0]
    elif t and (not isinstance(instance, pytypes[t])):
        raise AssertionError(f"{path or '/'}: expected {t}")
    if "pattern" in schema and isinstance(instance, str) and not re.search(schema["pattern"], instance):
        raise AssertionError(f"{path or '/'}: {instance!r} does not match pattern")
    if t == "object":
        for k in schema.get("required", []):
            if k not in instance:
                raise AssertionError(f"{path}/{k}: required")
        props = schema.get("properties", {})
        if schema.get("additionalProperties") is False:
            for k in instance:
                if k not in props:
                    raise AssertionError(f"{path}/{k}: additional property")
        for k, v in instance.items():
            if k in props:
                validate(v, props[k], root, f"{path}/{k}")
    if t == "array" and "items" in schema:
        for i, v in enumerate(instance):
            validate(v, schema["items"], root, f"{path}/{i}")


def check(instance, name):
    validate(instance, SCHEMA["$defs"][name], SCHEMA)


def load(name):
    return json.loads((HERE / "testdata" / name).read_text())


class ProbeContract(unittest.TestCase):
    def test_probe_sample_validates(self):
        check(load("probe-sample.json"), "probeResult")

    def test_probe_result_with_contract_validates(self):
        p = load("probe-sample.json")
        contract = {"contract_id": "3f860645de4c3363", "release": None,
                    "payloads": [{"event": "SessionStart", "kind": "violation", "field": "session_id"},
                                 {"event": None, "kind": "malformed", "field": None}]}
        check({**p, "contract": contract}, "probeResult")
        for bad in ({**contract, "release": "v0.1.0"}, {**contract, "payloads": [{"event": "x", "kind": "bad", "field": None}]},
                    {**contract, "extra": 1}):
            with self.subTest(bad=bad), self.assertRaises(AssertionError):
                check({**p, "contract": bad}, "probeResult")
        text = json.dumps({**p, "contract": contract})
        self.assertEqual(canary_bisect.parse_probe_output(text)["contract"], contract)

    def test_probe_json_copies_payload_classifications(self):
        script = str(ROOT / "scripts" / "harness-canary.sh")
        with tempfile.TemporaryDirectory() as d:
            d = pathlib.Path(d)
            (d / "checks.tsv").write_text("t0.payload-parse\tpass\tok\n")
            (d / "t0.json").write_text(json.dumps({"contract_id": "abc", "payloads": [
                {"file": "capture/tier0/1.stdin", "ok": True,
                 "contract": {"kind": "violation", "event": "SessionStart", "field": "session_id"}}]}))
            (d / "t1.json").write_text(json.dumps({"contract_id": "abc", "payloads": [
                {"file": "capture/tier1/1.json", "ok": True,
                 "contract": {"kind": "ok", "event": "PreToolUse", "field": None}}]}))
            src = f'HT_CANARY_SOURCE_ONLY=1 . {script}; canary_py probe-json {d}/checks.tsv auto {d}/t0.json {d}/missing.json {d}/t1.json 2>/dev/null'
            out = subprocess.run(["bash", "-c", src, script], capture_output=True, text=True, cwd=ROOT, check=True).stdout
            doc = json.loads(out)
            check(doc, "probeResult")
            self.assertEqual(doc["contract"]["payloads"], [
                {"event": "SessionStart", "kind": "violation", "field": "session_id"},
                {"event": "PreToolUse", "kind": "ok", "field": None}])
            self.assertEqual(doc["contract"]["contract_id"], "abc")
            src = f'HT_CANARY_SOURCE_ONLY=1 . {script}; canary_py probe-json {d}/checks.tsv auto {d}/missing.json 2>/dev/null'
            doc = json.loads(subprocess.run(["bash", "-c", src, script], capture_output=True, text=True, cwd=ROOT, check=True).stdout)
            self.assertNotIn("contract", doc)

    def test_canary_probe_sample_validates(self):
        check(load("canary-probe-sample.json"), "canaryProbe")

    def test_negative_cases(self):
        p, c = load("probe-sample.json"), load("canary-probe-sample.json")
        cases = [
            ("probeResult", {**p, "result": "maybe"}),
            ("probeResult", {**p, "failed_tier": 2}),
            ("probeResult", {**p, "extra": 1}),
            ("canaryProbe", {**c, "binary": "bin/codex"}),
            ("canaryProbe", {**c, "expected_admission": "maybe"}),
            ("canaryProbe", {**c, "version": "0.159"}),
        ]
        for name, doc in cases:
            with self.subTest(name=name, doc=doc):
                with self.assertRaises(AssertionError):
                    check(doc, name)

    def test_bisect_retry_policy(self):
        r = canary_bisect.retries_for
        self.assertEqual(r({"result": "fail", "checks": [], "failed_tier": 1}), 2)
        self.assertEqual(r({"result": "fail", "checks": [], "failed_tier": 0}), 1)
        self.assertEqual(r({"result": "fail", "checks": [], "failed_tier": None}), 1)
        self.assertEqual(r({"result": "pass", "checks": [], "failed_tier": None}), 0)
        self.assertEqual(r({"result": "infra", "checks": [], "failed_tier": None}), 1)

    def test_bisect_parses_probe_stdout(self):
        text = (HERE / "testdata" / "probe-sample.json").read_text()
        self.assertEqual(canary_bisect.parse_probe_output(text), load("probe-sample.json"))
        for bad in ("not json", '{"result":"fail","checks":[],"failed_tier":2}',
                    '{"result":"pass","checks":[],"failed_tier":null,"x":1}',
                    '{"result":"pass","checks":[{"id":"t0.a","status":"bogus"}],"failed_tier":null}'):
            with self.subTest(bad=bad):
                with self.assertRaises(ValueError):
                    canary_bisect.parse_probe_output(bad)

    def test_canary_script_writes_contract_files(self):
        script = str(ROOT / "scripts" / "harness-canary.sh")
        with tempfile.TemporaryDirectory() as d:
            subprocess.run(["bash", script, "--write-probe-files", d, "codex", "0.160.0", "/bin/sh"],
                           check=True, cwd=ROOT)
            doc = json.loads((pathlib.Path(d) / "canary-probe.json").read_text())
            check(doc, "canaryProbe")
            self.assertEqual(doc["expected_admission"], "schema-matched-or-optimistic")  # above verified_max
            self.assertTrue((pathlib.Path(d) / "help" / "codex.txt").is_file())
            self.assertTrue((pathlib.Path(d) / "help" / "codex-exec.txt").is_file())


if __name__ == "__main__":
    unittest.main()
