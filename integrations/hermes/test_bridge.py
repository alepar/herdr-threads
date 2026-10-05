"""Synthetic initialized native API/context and owned-child tests; no native/model PASS.

Mutations caught: extra hooks/content forwarding, assumed parent/cache roles,
inline/replaced readers, lost contextvars, stale publication, ignored child
caps/deadlines/ack mismatch, and process-local lifecycle authority.
"""
import contextvars
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import threading
import time
import types
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).with_name("__init__.py")
DATA = Path(__file__).with_name("testdata")
SLOT = "_herdr_threads_hermes_reader_schema1"
PROFILE = contextvars.ContextVar("synthetic_native_profile", default="wrong-profile")
VERSION = dict(base_version="0.21.5", derived_version="0.21.5+3962.g37daf85",
               commit="37daf85b2ad0ee50ed45d7234dc47b7fa24cec09", distance=3962,
               dirty=False, source="git", branch="main", commit_date=None, distribution=None)

CHILD = r'''
import json,os,signal,sys,time
from pathlib import Path
root=Path(sys.argv[sys.argv.index('--state-dir')+1])
(root/'pid').write_text(str(os.getpid()))
request=json.load(sys.stdin)
with (root/'requests').open('a') as f: f.write(json.dumps({'argv':sys.argv[1:],'request':request})+'\n')
mode=(root/'mode').read_text()
fixtures=json.loads((root/'results.json').read_text())
if mode=='ignore_term': signal.signal(signal.SIGTERM,signal.SIG_IGN); time.sleep(60)
if mode=='hang': time.sleep(60)
if mode=='stderr': sys.stderr.buffer.write(b'x'*9000); sys.stderr.flush(); time.sleep(60)
if mode=='overflow': sys.stdout.buffer.write(b'x'*9000); sys.stdout.flush(); time.sleep(60)
if mode=='nonzero': sys.exit(7)
if mode=='invalid': print('{'); sys.exit(0)
if mode=='duplicate': print('{"context":null,"context":"bad","lifecycle_ack":null}'); sys.exit(0)
if mode=='context_overflow': print(json.dumps({'context':'é'*2049,'lifecycle_ack':None})); sys.exit(0)
if mode=='slow': time.sleep(.3)
if request['callback']!='pre_llm_call':
    observer=fixtures['observer']
    observer['lifecycle_ack']['event_id']=request['event_id']
    observer['lifecycle_ack']['session_id']=request['session_id']
    print(json.dumps(observer)); sys.exit(0)
journal=root/'journal'
state=json.loads(journal.read_text()) if journal.exists() else {'events':{},'sessions':[]}
event=request['event_id']; session=request['session_id']
if event not in state['events']:
    selected='current' if session in state['sessions'] else 'startup'
    state['sessions'].append(session)
    state['events'][event]=selected
    journal.write_text(json.dumps(state))
ack={'event_id':event,'session_id':session,'mode':state['events'][event]}
if mode=='mismatch': ack['event_id']='wrong'
if mode=='bad_mode': ack['mode']='resume'
if mode=='extra_ack': ack['secret']='PRIVATE'
result=fixtures['context']
result['lifecycle_ack']=ack
if mode=='context_max': result['context']='é'*2048
if mode=='extra': result=fixtures['malformed']
if mode=='mismatch':
    result=fixtures['mismatch']
    result['lifecycle_ack']['session_id']=session
print(json.dumps(result,ensure_ascii=False))
'''


class BridgeTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(dir=os.environ.get("HT_TASK_TMP"), prefix="bridge-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.home = self.root / "home Ω"
        self.asset = self.home / "plugins" / "herdr-threads"
        self.asset.mkdir(parents=True)
        self.native = self.root / "native"
        (self.native / "hermes_cli").mkdir(parents=True)
        self.state = self.root / "state Ω"
        self.state.mkdir()
        self.child = self.root / "owned child Ω"
        self.child.write_text("#!" + sys.executable + "\n" + CHILD)
        self.child.chmod(0o700)
        (self.state / "mode").write_text("success")
        self.settings = dict(schema_version=1, rust_executable=str(self.child),
                             state_root=str(self.state), host_endpoint=str(self.root / "host.sock"),
                             bridge_schema_version=1, installation_token="owned-token")
        (self.asset / "bridge_config.json").write_text(json.dumps(self.settings))
        (self.state / "results.json").write_text((DATA / "results.json").read_text())
        self.config = {"plugins": {"hook_callback_timeout": 30}, "secret": "PRIVATE CONFIG"}
        self.read_error = None
        self.block_read = False
        self.rotate_profile = False
        self.entered = threading.Event()
        self.release = threading.Event()
        self.reads = []
        self.version_reads = 0
        self.modules = {}
        for name in ("hermes_bootstrap", "hermes_constants", "hermes_cli", "hermes_cli.config",
                     "hermes_cli.profiles", "hermes_cli.version_info", "hermes_cli.plugins"):
            module = types.ModuleType(name)
            relative = Path(*name.split(".")).with_suffix(".py")
            if name == "hermes_cli":
                relative = Path("hermes_cli/__init__.py")
            origin = str(self.native / relative)
            Path(origin).write_text("# synthetic already initialized native module\n")
            module.__file__ = origin
            module.__spec__ = importlib.util.spec_from_file_location(name, origin)
            self.modules[name] = module
        class FailedConfigRead(dict):
            read_error = RuntimeError("PRIVATE READ ERROR")
        self.failed = FailedConfigRead
        self.modules["hermes_cli.config"].FailedConfigRead = FailedConfigRead
        self.modules["hermes_cli.config"].load_config_readonly = self.read_config
        self.version = types.SimpleNamespace(**VERSION)
        self.modules["hermes_cli.version_info"].get_version_info = self.read_version
        self.modules["hermes_constants"].get_hermes_home = lambda: Path(PROFILE.get())
        self.modules["hermes_cli.profiles"].current_profile_name = lambda default=None: "default"
        self.modules["hermes_cli.profiles"].resolve_profile_env = lambda name: str(self.home)
        class PluginContext:
            __module__ = "hermes_cli.plugins"
            def __init__(self):
                self.hooks = {}
                self.unloads = []
            def register_hook(self, name, callback):
                self.hooks[name] = callback
            def on_unload(self, callback):
                self.unloads.append(callback)
        self.context_type = PluginContext
        self.modules["hermes_cli.plugins"].PluginContext = PluginContext
        self.env = patch.dict(os.environ, {"HOME": str(self.root), "CLAUDE_CONFIG_DIR": str(self.root / "claude"),
                                          "CODEX_HOME": str(self.root / "codex"), "PYTHONDONTWRITEBYTECODE": "1"})
        self.env.start()
        self.addCleanup(self.env.stop)
        self.native_patch = patch.dict(sys.modules, self.modules)
        self.native_patch.start()
        self.addCleanup(self.native_patch.stop)
        sys.modules.pop(SLOT, None)
        self.contexts = []
        self.addCleanup(self.stop_readers)
        self.callbacks = json.loads((DATA / "callbacks.json").read_text())

    def stop_readers(self):
        self.release.set()
        for context in self.contexts:
            for callback in context.unloads:
                callback()
        slot = sys.modules.get(SLOT)
        reader = getattr(slot, "reader", None)
        if reader is not None:
            reader.thread.join(2)
            self.assertFalse(reader.thread.is_alive(), "synthetic reader leaked")
        if (self.state / "pid").exists():
            pid = int((self.state / "pid").read_text())
            with self.assertRaises(ProcessLookupError):
                os.kill(pid, 0)

    def read_config(self):
        self.reads.append(PROFILE.get())
        self.entered.set()
        if self.block_read:
            self.release.wait(3)
        if self.read_error:
            raise self.read_error
        if self.rotate_profile:
            PROFILE.set("different native profile")
        return self.config

    def read_version(self):
        self.version_reads += 1
        return self.version

    def load(self, wait=True):
        module = types.ModuleType("synthetic_owned_plugin")
        module.__file__ = str(self.asset / "__init__.py")
        if SOURCE.exists():
            exec(compile(SOURCE.read_text(), module.__file__, "exec"), module.__dict__)
        self.assertTrue(callable(getattr(module, "register", None)), "actual bridge registration is missing")
        ctx = self.context_type()
        self.contexts.append(ctx)
        token = PROFILE.set(str(self.home))
        try:
            module.register(ctx)
        finally:
            PROFILE.reset(token)
        self.module = module
        self.ctx = ctx
        if wait:
            self.settle()
        return ctx

    def settle(self):
        limit = time.monotonic() + 2
        while time.monotonic() < limit:
            slot = sys.modules.get(SLOT)
            reader = getattr(slot, "reader", None)
            if reader and reader.snapshot() is not None:
                return reader
            time.sleep(.002)
        self.fail("native observation did not become ready")

    def request(self, callback="pre_llm_call", payload=None):
        return self.ctx.hooks[callback](**(self.callbacks["qualified"] if payload is None else payload))

    def requests(self):
        path = self.state / "requests"
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def reader(self):
        return sys.modules[SLOT].reader

    def refresh(self):
        reader = self.reader()
        reader.next_refresh = 0
        reader.wake.set()
        return reader

    def test_bridge_callback_privacy_role_cache_deadlines_and_owned_child_reaping(self):
        self.load()
        self.assertEqual(set(self.ctx.hooks), {"pre_llm_call", "post_tool_call", "on_session_start", "on_session_reset"})
        self.assertEqual(self.request(), {"context": "bounded cooperative context"})
        captured = self.requests()[0]
        self.assertEqual(captured["argv"], ["--state-dir", str(self.state), "--host-endpoint", self.settings["host_endpoint"], "hook", "hermes"])
        request = captured["request"]
        self.assertEqual(request["bridge_schema_contract_id"], self.callbacks["bridge_schema_contract_id"])
        self.assertEqual(request["parent_session_id"], "")
        self.assertEqual(request["runtime_identity"]["commit"], VERSION["commit"])
        self.assertTrue(request["runtime_identity"]["key"].startswith("build:"))
        self.assertEqual(request["runtime_identity"]["distance"], 3962)
        self.assertEqual(set(request), {"schema_version", "callback", "platform", "session_id", "parent_session_id", "task_id", "turn_id", "tool_call_id", "api_request_id", "event_id", "observation_order", "started_at", "deadline_at", "reset_reason", "runtime_identity", "identity_unavailable_reason", "shape", "bridge_schema_contract_id"})
        self.assertNotIn("PRIVATE", json.dumps(captured))
        self.assertLessEqual(request["deadline_at"] - request["started_at"], 1200)
        self.assertIsNone(self.request("post_tool_call", self.callbacks["tool"]))
        self.assertEqual(len(self.requests()), 2)
        self.assertNotIn("PRIVATE", json.dumps(self.requests()))
        (self.state / "mode").write_text("hang")
        reader = self.reader()
        with reader.lock:
            reader.observed_timeout = .4
        start = time.monotonic()
        self.assertIsNone(self.request())
        self.assertLess(time.monotonic() - start, .7)
        self.assertEqual(len(self.requests()), 3, "owned hang child must have received the envelope")
        pid = int((self.state / "pid").read_text())
        with self.assertRaises(ProcessLookupError):
            os.kill(pid, 0)

    def test_unknown_parent_child_platform_and_bad_ids_never_checkin(self):
        self.load()
        for parent in (None, 1, True, "x" * 257, "bad\x85id"):
            payload = {**self.callbacks["qualified"], "parent_session_id": parent}
            self.assertIsNone(self.request(payload=payload))
        payload = dict(self.callbacks["qualified"])
        del payload["parent_session_id"]
        self.assertIsNone(self.request(payload=payload))
        child = self.request(payload=self.callbacks["child"])
        self.assertIn("never check in", child["context"])
        self.assertIn("forbidden to subagents", child["context"])
        for field in ("session_id", "turn_id", "task_id", "tool_call_id", "api_request_id"):
            self.assertIsNone(self.request(payload={**self.callbacks["qualified"], field: "bad\nvalue"}))
        self.assertIsNone(self.request(payload={**self.callbacks["qualified"], "platform": "gateway"}))
        self.assertEqual(self.requests(), [])
        self.assertEqual(self.request(payload={**self.callbacks["qualified"], "platform": ""}), {"context": "bounded cooperative context"})

    def test_observer_cache_exact_key_expiry_capacity_busy_and_reset(self):
        self.load()
        self.assertIsNone(self.request("post_tool_call", self.callbacks["tool"]))
        self.assertEqual(self.requests(), [])
        self.request()
        bridge = self.ctx.hooks["pre_llm_call"].__self__
        for tool in ({**self.callbacks["tool"], "turn_id":"other"}, {**self.callbacks["tool"], "session_id":"other"}):
            self.assertIsNone(self.request("post_tool_call", tool))
        self.assertEqual(len(self.requests()), 1)
        with bridge.role_lock:
            self.assertIsNone(self.request("post_tool_call", self.callbacks["tool"]))
            self.assertIsNone(self.request())
        with bridge.role_lock:
            bridge.roles[("session-Ω", "turn-1")] = ("top", time.monotonic() - 601)
        self.request("post_tool_call", self.callbacks["tool"])
        self.assertEqual(len(self.requests()), 1)
        for i in range(129):
            bridge.remember_role("s" + str(i), "t", "top", time.monotonic())
        self.assertLessEqual(len(bridge.roles), 128)
        self.request("post_tool_call", {"session_id":"s0", "turn_id":"t"})
        self.assertEqual(len(self.requests()), 1)
        bridge.remember_role("session-Ω", "turn-1", "top", time.monotonic())
        bridge.remember_role("keep", "t", "top", time.monotonic())
        self.assertIsNone(self.request("on_session_reset", self.callbacks["reset"]))
        self.request("post_tool_call", self.callbacks["tool"])
        self.assertEqual(len(self.requests()), 2)
        self.assertIn(("keep", "t"), bridge.roles)
        self.assertEqual(self.requests()[-1]["request"]["reset_reason"], "new_session")
        self.assertIsNone(self.request("on_session_start", {"session_id":"startup", "user_message":"PRIVATE"}))
        self.assertEqual(len(self.requests()), 3)

    def test_effective_config_resolver_semantics_quality_and_staleness(self):
        self.load()
        for value, expected in ((None,30), ("bad",30), (-1,30), (0,0), (.2,.2), (.3,.3), (900,600)):
            self.config = {"plugins":{"hook_callback_timeout":value}}
            self.entered.clear()
            self.refresh()
            self.assertTrue(self.entered.wait(2))
            reader = self.settle()
            self.assertEqual(reader.snapshot()["timeout_seconds"], expected)
        for config in (self.failed(self.config), {"plugins":{"hook_callback_timeout":float("nan")}}, {"plugins":{"hook_callback_timeout":float("inf")}}, []):
            self.config = config
            self.refresh()
            time.sleep(.03)
            self.assertIsNone(self.request())
        self.assertEqual(self.requests(), [])
        self.config = {"plugins":{"hook_callback_timeout":30}}
        self.refresh()
        reader = self.settle()
        with reader.lock:
            reader.observed_at = time.monotonic() - 6
        self.assertIsNone(self.request())
        self.assertEqual(self.requests(), [])

    def test_contextvars_hung_single_reader_pending_and_invalidated_late_completion(self):
        self.block_read = True
        self.load(wait=False)
        self.assertTrue(self.entered.wait(2))
        self.assertEqual(self.reads, [str(self.home)])
        first_reader = self.reader()
        start = time.monotonic()
        for _ in range(20):
            self.assertIsNone(self.request())
        self.assertLess(time.monotonic() - start, .2)
        self.assertEqual(self.requests(), [])
        self.assertEqual(len(self.reads), 1)
        self.ctx.unloads[0]()
        self.assertTrue(first_reader.thread.is_alive())
        self.load(wait=False)
        self.assertIs(self.reader(), first_reader)
        self.assertIsNone(self.request())
        self.assertEqual(len(self.reads), 1)
        self.release.set()
        first_reader.thread.join(2)
        self.assertFalse(first_reader.thread.is_alive())
        self.assertIsNone(first_reader.snapshot())
        self.assertEqual(self.requests(), [])

    def test_startup_identity_is_retained_across_source_and_config_changes(self):
        self.load()
        self.request()
        original = self.requests()[-1]["request"]["runtime_identity"]
        self.version = types.SimpleNamespace(**{**VERSION,"commit":"a"*40,"dirty":True})
        self.modules["hermes_cli.version_info"].__file__ = "later changed source path"
        self.config = {"plugins":{"hook_callback_timeout":0}}
        self.refresh()
        self.settle()
        self.request(payload={**self.callbacks["qualified"],"turn_id":"turn-2"})
        self.assertEqual(self.requests()[-1]["request"]["runtime_identity"], original)
        self.assertEqual(self.version_reads, 1)
        self.assertEqual(self.reader().snapshot()["provenance"], "official_effective_config_observation")
        self.assertGreaterEqual(self.reader().snapshot()["age_seconds"], 0)
        self.assertEqual(self.reader().identity_provenance, "startup_captured_identity")

    def test_read_exception_is_private_and_no_implicit_default(self):
        self.read_error = RuntimeError("PRIVATE CONFIG ERROR")
        self.load(wait=False)
        self.assertTrue(self.entered.wait(2))
        time.sleep(.03)
        self.assertIsNone(self.request())
        self.assertEqual(self.requests(), [])
        reader = self.reader()
        self.assertEqual(reader.quality, "read_error")
        self.assertIsNone(reader.fallback)
        self.assertNotIn("PRIVATE", repr(reader.__dict__))

    def test_child_results_errors_caps_and_nonblocking_generation(self):
        self.load()
        for mode in ("nonzero", "invalid", "duplicate", "context_overflow", "mismatch", "bad_mode", "extra_ack", "extra", "overflow", "stderr", "ignore_term"):
            with self.subTest(mode=mode):
                (self.state / "mode").write_text(mode)
                self.assertIsNone(self.request())
                pid = int((self.state / "pid").read_text())
                with self.assertRaises(ProcessLookupError):
                    os.kill(pid, 0)
        (self.state / "mode").write_text("slow")
        outcome = []
        worker = threading.Thread(target=lambda: outcome.append(self.request()))
        worker.start()
        self.addCleanup(worker.join, 2)
        start = time.monotonic()
        while not (self.state / "mode").exists() or len(self.requests()) < 12:
            if time.monotonic() - start > 1: self.fail("child not started")
            time.sleep(.002)
        self.assertIsNone(self.request(payload={**self.callbacks["qualified"],"turn_id":"newer"}))
        worker.join(2)
        self.assertEqual(outcome, [None])

    def test_deadlines_disabled_small_unknown_and_busy_snapshot(self):
        self.load()
        reader = self.reader()
        for value in (0,.2,.1):
            with reader.lock:
                reader.observed_timeout = value
            result = self.request()
            self.assertEqual(result, {"context":"bounded cooperative context"} if value == 0 else None)
        self.assertEqual(len(self.requests()), 1)
        request = self.requests()[0]["request"]
        self.assertLessEqual(request["deadline_at"] - request["started_at"],1200)
        with reader.lock:
            self.assertIsNone(self.request())
        self.assertEqual(len(self.requests()), 1)

    def test_lifecycle_is_rust_owned_exact_replay_restart_and_failed_first_attach(self):
        self.load()
        (self.state / "mode").write_text("nonzero")
        self.assertIsNone(self.request())
        (self.state / "mode").write_text("success")
        self.assertEqual(self.request(), {"context":"bounded cooperative context"})
        first = self.requests()[-1]["request"]
        self.assertEqual(self.request(), {"context":"bounded cooperative context"})
        replay = self.requests()[-1]["request"]
        self.assertEqual(first["event_id"], replay["event_id"])
        self.assertGreater(replay["observation_order"]["sequence"], first["observation_order"]["sequence"])
        self.ctx.unloads[0]()
        self.reader().thread.join(2)
        self.load()
        self.request(payload={**self.callbacks["qualified"],"turn_id":"turn-2"})
        journal = json.loads((self.state / "journal").read_text())
        self.assertEqual(set(journal["events"].values()), {"startup", "current"})
        self.assertEqual(first["event_id"], self.requests()[1]["request"]["event_id"])

    def test_reload_cannot_spawn_successor_while_unloaded_bridge_owns_live_child(self):
        self.load()
        first = self.reader()
        (self.state / "mode").write_text("slow")
        outcome = []
        worker = threading.Thread(target=lambda: outcome.append(self.request()))
        worker.start()
        self.addCleanup(worker.join, 2)
        limit = time.monotonic() + 1
        while not self.requests() and time.monotonic() < limit:
            time.sleep(.002)
        self.assertEqual(len(self.requests()), 1)
        self.ctx.unloads[0]()
        first.thread.join(2)
        self.load(wait=False)
        if self.reader() is not first:
            self.settle()
        self.assertIsNone(self.request())
        self.assertEqual(len(self.requests()), 1)
        worker.join(2)
        self.assertEqual(outcome, [None])

    def test_valid_context_at_utf8_byte_boundary_is_returned(self):
        self.load()
        (self.state / "mode").write_text("context_max")
        self.assertEqual(self.request(), {"context":"é"*2048})

    def test_strict_owned_config_refuses_bad_paths_extra_fields_and_overflow(self):
        for settings in ({**self.settings, "rust_executable":"relative"},
                         {**self.settings, "extra":"PRIVATE CONFIG"},
                         {**self.settings, "schema_version":True},
                         {**self.settings, "host_endpoint":"bad\npath"},
                         {**self.settings, "installation_token":"x"*257}):
            (self.asset / "bridge_config.json").write_text(json.dumps(settings))
            self.load(wait=False)
            self.assertEqual(self.ctx.hooks, {})
            self.assertEqual(self.reads, [])
        (self.asset / "bridge_config.json").write_text(" " * 65537)
        self.load(wait=False)
        self.assertEqual(self.ctx.hooks, {})
        self.assertEqual(self.reads, [])
        self.assertEqual(self.requests(), [])

    def test_changed_native_profile_during_refresh_cannot_deliver_under_startup_identity(self):
        self.load()
        self.rotate_profile = True
        self.entered.clear()
        self.refresh()
        self.assertTrue(self.entered.wait(2))
        limit = time.monotonic() + 1
        while self.reader().pending and time.monotonic() < limit:
            time.sleep(.002)
        self.assertIsNone(self.request())
        self.assertEqual(self.requests(), [])

    def test_incomplete_git_startup_descriptor_cannot_deliver_context(self):
        for field in ("commit", "distance"):
            self.version = types.SimpleNamespace(**{**VERSION,field:None})
            self.load(wait=False)
            time.sleep(.03)
            self.assertIsNone(self.request())
            self.assertEqual(self.requests(), [])
            self.ctx.unloads[0]()
            self.reader().thread.join(2)

    def assert_startup_descriptor_refused(self, changes):
        (self.state / "requests").unlink(missing_ok=True)
        (self.state / "pid").unlink(missing_ok=True)
        self.version = types.SimpleNamespace(**{**VERSION, **changes})
        self.load(wait=False)
        reader = self.reader()
        try:
            limit = time.monotonic() + 2
            while reader.pending and time.monotonic() < limit:
                time.sleep(.002)
            self.assertFalse(reader.pending, "synthetic startup capture did not complete")
            self.assertIsNone(self.request())
            self.assertEqual(self.requests(), [], "malformed identity reached owned child")
            self.assertFalse((self.state / "pid").exists(), "malformed identity started owned child")
        finally:
            self.ctx.unloads[0]()
            reader.thread.join(2)
            self.assertFalse(reader.thread.is_alive(), "synthetic reader leaked")

    def test_null_required_startup_versions_cannot_deliver_context(self):
        for field in ("base_version", "derived_version"):
            with self.subTest(field=field):
                self.assert_startup_descriptor_refused({field: None})

    def test_undeclared_startup_source_cannot_deliver_context(self):
        for source in ("invented", "unknown", "commit_build", "", None, 7):
            with self.subTest(source=source):
                self.assert_startup_descriptor_refused({"source": source})

    def test_required_startup_version_bounds_cannot_deliver_context(self):
        for field in ("base_version", "derived_version"):
            for value in ("", 7, "x" * 129, "é" * 65, "0.21.5\n"):
                with self.subTest(field=field, value=value):
                    self.assert_startup_descriptor_refused({field: value})

    def test_declared_non_git_sources_with_optional_fields_deliver_context(self):
        for source in ("build", "commit-build", "ci", "docker", "fallback", "local", "nix"):
            with self.subTest(source=source):
                self.version = types.SimpleNamespace(**{**VERSION, "source": source,
                                                        "commit": None, "distance": None,
                                                        "base_version": "x" * 128,
                                                        "derived_version": "é" * 64})
                self.load()
                try:
                    before = len(self.requests())
                    self.assertEqual(self.request(), {"context": "bounded cooperative context"})
                    self.assertEqual(len(self.requests()), before + 1)
                    identity = self.requests()[-1]["request"]["runtime_identity"]
                    self.assertEqual(identity["source"], source)
                    self.assertIsNone(identity["commit"])
                    self.assertIsNone(identity["distance"])
                    self.assertEqual(identity["base_version"], "x" * 128)
                    self.assertEqual(identity["derived_version"], "é" * 64)
                finally:
                    self.ctx.unloads[0]()
                    self.reader().thread.join(2)
                    self.assertFalse(self.reader().thread.is_alive(), "synthetic reader leaked")

    def test_missing_initialized_native_api_does_not_start_reader(self):
        del self.modules["hermes_cli.config"].load_config_readonly
        self.load(wait=False)
        self.assertEqual(self.ctx.hooks, {})
        self.assertEqual(self.reads, [])
        self.assertEqual(self.requests(), [])

    def test_newer_entry_during_result_validation_cannot_publish_old_context(self):
        self.load()
        parse = self.module.strict_json
        def validation_race(raw):
            self.assertIsNone(self.request(payload={**self.callbacks["qualified"], "turn_id":"newer"}))
            return parse(raw)
        with patch.object(self.module, "strict_json", validation_race):
            self.assertIsNone(self.request())
        self.assertEqual(len(self.requests()), 1)

    def test_unreaped_child_blocks_a_successor_until_owned_process_has_exited(self):
        self.load()
        (self.state / "mode").write_text("ignore_term")
        with self.reader().lock:
            self.reader().observed_timeout = .5
        children = []
        original = self.module.subprocess.Popen
        class DelayedKill(original):
            def __init__(self, *args, **kwargs):
                super().__init__(*args, **kwargs)
                children.append(self)
            def kill(self):
                pass  # synthetic OS kill delay; real owned process remains observable
        try:
            with patch.object(self.module.subprocess, "Popen", DelayedKill):
                self.assertIsNone(self.request())
                self.assertEqual(len(children), 1)
                self.assertIsNone(children[0].poll(), "synthetic delayed-kill child must still be alive")
                self.assertIsNone(self.request(payload={**self.callbacks["qualified"],"turn_id":"successor"}))
                self.assertEqual(len(children), 1, "a successor was spawned before owned child was reaped")
                self.assertEqual(len(self.requests()), 1)
        finally:
            for child in children:
                original.kill(child)
                original.wait(child, timeout=2)

    def test_refresh_cadence_and_failed_config_unknown_fallback(self):
        self.load()
        self.assertEqual(len(self.reads), 1)
        self.assertTrue(self.reader().next_refresh >= self.reader().observed_at + 1)
        time.sleep(.1)
        self.assertEqual(len(self.reads), 1)
        self.config = self.failed({"plugins":{"hook_callback_timeout":0}})
        self.entered.clear()
        self.assertTrue(self.entered.wait(2))
        limit = time.monotonic() + 1
        while self.reader().pending and time.monotonic() < limit:
            time.sleep(.002)
        self.assertEqual(self.reader().quality, "failed_config_read")
        self.assertIsNone(self.reader().fallback)
        self.assertIsNone(self.request())
        self.assertEqual(self.requests(), [])

    def test_unload_during_owned_child_blocks_late_context(self):
        self.load()
        (self.state / "mode").write_text("slow")
        outcome = []
        worker = threading.Thread(target=lambda: outcome.append(self.request()))
        worker.start()
        self.addCleanup(worker.join, 2)
        limit = time.monotonic() + 1
        while not self.requests() and time.monotonic() < limit:
            time.sleep(.002)
        self.assertEqual(len(self.requests()), 1)
        self.ctx.unloads[0]()
        worker.join(2)
        self.assertEqual(outcome, [None])

    def test_unload_contended_publication_lock_invalidates_cached_snapshot(self):
        self.load()
        reader = self.reader()
        with reader.lock:
            self.ctx.unloads[0]()
        self.assertIsNone(reader.snapshot())
        self.assertIsNone(self.request())
        self.assertEqual(self.requests(), [])

    def test_pending_callback_does_not_seed_a_qualified_observer_association(self):
        self.block_read = True
        self.load(wait=False)
        self.assertTrue(self.entered.wait(2))
        self.assertIsNone(self.request())
        self.release.set()
        self.settle()
        self.assertIsNone(self.request("post_tool_call", self.callbacks["tool"]))
        self.assertEqual(self.requests(), [])

    def test_origin_mismatch_and_unavailable_api_do_not_fabricate_identity(self):
        self.modules["hermes_cli.config"].__spec__.origin = str(self.root / "foreign.py")
        self.load(wait=False)
        time.sleep(.03)
        self.assertIsNone(self.request())
        self.assertEqual(self.requests(), [])
        self.assertIsNone(self.reader().identity)


if __name__ == "__main__":
    unittest.main()
