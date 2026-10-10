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
import signal
import subprocess
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
if mode=='lifetime_hold':
    limit=time.monotonic()+3
    while not (root/'release_child').exists() and time.monotonic()<limit: time.sleep(.002)
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


class PublicationLock:
    """Observe publication by the worker without replacing its behavior.

    A provider return alone is insufficient: the worker must publish pending=False
    under its own lock for that exact capture/read iteration.
    """
    def __init__(self, reader):
        self.reader = reader
        self.lock = reader.lock
        self.counts = {"capture": 0, "timeout": 0}
        self.entered = {}
        self.completed = {}
        self.current = None
        for kind, name in (("capture", "capture"), ("timeout", "observe_timeout")):
            original = getattr(reader.provider, name)
            setattr(reader.provider, name, self.observe(kind, original))

    def observe(self, kind, original):
        def call():
            with self.lock:
                self.counts[kind] += 1
                key = (kind, self.counts[kind])
                self.current = (key, False)
                self.entered[key] = True
            try:
                return original()
            finally:
                with self.lock:
                    self.current = (key, True)
        return call

    def acquire(self, *args, **kwargs):
        return self.lock.acquire(*args, **kwargs)

    def release(self):
        reader = self.reader
        if (threading.current_thread() is reader.thread and self.current is not None
                and self.current[1] and not reader.pending):
            key = self.current[0]
            self.completed.setdefault(key, (reader.quality, reader.observed_timeout,
                                            reader.observed_at, reader.identity))
        self.lock.release()

    def __enter__(self):
        self.acquire()
        return self

    def __exit__(self, *args):
        self.release()


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
        self.readers = []
        self.children = []
        self.clock = None
        self.addCleanup(self.stop_readers)
        self.callbacks = json.loads((DATA / "callbacks.json").read_text())

    def stop_readers(self):
        self.release.set()
        for context in self.contexts:
            for callback in context.unloads:
                callback()
        alive = []
        for reader in self.readers:
            reader.close()
            reader.thread.join(2)
            if reader.thread.is_alive():
                alive.append(reader)
        for child in self.children:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGTERM)
                try:
                    child.wait(timeout=.1)
                except subprocess.TimeoutExpired:
                    os.killpg(child.pid, signal.SIGKILL)
                    child.wait(timeout=2)
            self.assertIsNotNone(child.returncode, "synthetic child was not reaped")
        self.assertEqual(alive, [], "synthetic reader leaked")
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
        if self.clock is not None:
            # Change only this executed synthetic module; waits/joins keep real time.
            module.time = types.SimpleNamespace(monotonic=lambda: self.clock, time=time.time)
        children = self.children
        class OwnedChild(subprocess.Popen):
            def __init__(self, *args, **kwargs):
                kwargs["start_new_session"] = True
                kwargs["env"] = {key: os.environ[key] for key in (
                    "HOME", "CLAUDE_CONFIG_DIR", "CODEX_HOME", "PYTHONDONTWRITEBYTECODE",
                    "HT_LEAK_RUN_ID") if key in os.environ}
                kwargs["env"]["HT_TEST_OWNER"] = str(os.getpid())
                self.fixture_live_pids = [child.pid for child in children if child.poll() is None]
                super().__init__(*args, **kwargs)
                self.fixture_started_at = time.time()
                children.append(self)
        module.subprocess = types.SimpleNamespace(Popen=OwnedChild, PIPE=subprocess.PIPE,
                                                  TimeoutExpired=subprocess.TimeoutExpired)
        initialize = module.Reader.__init__
        def tracked_initialize(reader, provider):
            initialize(reader, provider)
            reader.lock = PublicationLock(reader)
            self.readers.append(reader)
        module.Reader.__init__ = tracked_initialize
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
        reader = self.reader()
        self.await_completion(reader)
        self.assertIsNotNone(reader.snapshot(), "completed native observation is unusable")
        return reader

    def await_entry(self, reader, kind="timeout", iteration=1, timeout=2):
        limit = time.monotonic() + timeout
        while time.monotonic() < limit:
            if reader.lock.acquire(timeout=.01):
                try:
                    if (kind, iteration) in reader.lock.entered:
                        return
                finally:
                    reader.lock.release()
            time.sleep(.002)
        self.fail(f"target {kind} iteration {iteration} did not enter")

    def await_completion(self, reader, kind="timeout", iteration=1, quality="ok", timeout=2):
        self.await_entry(reader, kind, iteration, timeout)
        limit = time.monotonic() + timeout
        while time.monotonic() < limit:
            if reader.lock.acquire(timeout=.01):
                try:
                    result = reader.lock.completed.get((kind, iteration))
                    if result is not None:
                        self.assertFalse(reader.pending, "target observation is still pending")
                        self.assertEqual(result[0], quality)
                        self.assertEqual(reader.quality, quality)
                        self.assertEqual(reader.observed_timeout, result[1])
                        self.assertEqual(reader.observed_at, result[2])
                        self.assertIs(reader.identity, result[3])
                        return result
                finally:
                    reader.lock.release()
            time.sleep(.002)
        self.fail(f"target {kind} iteration {iteration} did not publish completion")

    def request(self, callback="pre_llm_call", payload=None):
        return self.ctx.hooks[callback](**(self.callbacks["qualified"] if payload is None else payload))

    def test_startup_and_observed_timeout_facts_reach_real_child_without_recapture(self):
        self.load()
        self.assertIsNotNone(self.request())
        request = self.requests()[-1]["request"]
        self.assertIn("startup_capture", request, "actual startup facts never reach Rust admission")
        capture = request["startup_capture"]
        self.assertEqual(capture["identity_provenance"], "startup_captured_identity")
        self.assertEqual(capture["api_contract"], "initialized_plugin_context_schema1")
        self.assertEqual(capture["profile"], "default")
        self.assertEqual(capture["physical_home"], str(self.home.resolve()))
        self.assertEqual(set(capture["module_origins"]), {"hermes_bootstrap", "hermes_constants", "hermes_cli.profiles", "hermes_cli.version_info", "hermes_cli.config", "hermes_cli.plugins"})
        observation = request["timeout_observation"]
        self.assertEqual(observation["quality"], "ok")
        self.assertIsNone(observation["fallback_kind"])
        self.assertEqual(observation["provenance"], "official_effective_config_observation")
        self.assertEqual(observation["timeout_seconds"], 30.0)
        self.assertGreaterEqual(observation["age_seconds"], 0)
        self.assertLessEqual(observation["age_seconds"], 5)
        self.assertEqual(request["role_association"], {"role":"top", "provenance":"explicit_parent", "session_id":"session-Ω", "turn_id":"turn-1"})
        self.request("post_tool_call", self.callbacks["tool"])
        tool = self.requests()[-1]["request"]
        self.assertEqual(tool["role_association"], {"role":"top", "provenance":"qualified_pre_llm_cache", "session_id":"session-Ω", "turn_id":"turn-1"})
        self.assertEqual(tool["startup_capture"], capture)
        self.assertEqual(self.version_reads, 1)

    def requests(self):
        path = self.state / "requests"
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def reader(self):
        return sys.modules[SLOT].reader

    def refresh(self, quality="ok"):
        reader = self.reader()
        with reader.lock:
            iteration = reader.lock.counts["timeout"] + 1
            reader.next_refresh = 0
        reader.wake.set()
        self.await_completion(reader, iteration=iteration, quality=quality)
        return reader

    def test_bridge_callback_privacy_role_cache_deadlines_and_owned_child_reaping(self):
        self.load()
        self.assertEqual(set(self.ctx.hooks), {"pre_tool_call", "pre_llm_call", "post_tool_call",
                                              "on_session_start", "on_session_reset"})
        self.assertEqual(self.request(), {"context": "bounded cooperative context"})
        captured = self.requests()[0]
        self.assertEqual(captured["argv"], ["--state-dir", str(self.state), "--host-endpoint", self.settings["host_endpoint"], "hook", "hermes"])
        request = captured["request"]
        self.assertEqual(request["bridge_schema_contract_id"], self.callbacks["bridge_schema_contract_id"])
        self.assertEqual(request["parent_session_id"], "")
        self.assertEqual(request["runtime_identity"]["commit"], VERSION["commit"])
        self.assertTrue(request["runtime_identity"]["key"].startswith("build:"))
        self.assertEqual(request["runtime_identity"]["distance"], 3962)
        self.assertEqual(set(request), {"schema_version", "callback", "platform", "session_id", "parent_session_id", "task_id", "turn_id", "tool_call_id", "api_request_id", "event_id", "observation_order", "started_at", "deadline_at", "reset_reason", "runtime_identity", "identity_unavailable_reason", "shape", "bridge_schema_contract_id", "startup_capture", "timeout_observation", "role_association"})
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
        # Native delegate_task children call with platform "subagent" (Hermes 0.21.5).
        child = self.request(payload=self.callbacks["child"])
        self.assertIn("never check in", child["context"])
        self.assertIn("forbidden to subagents", child["context"])
        self.assertEqual(self.request(payload={**self.callbacks["child"], "platform": "cli"}), child)
        self.assertIsNone(self.request(payload={**self.callbacks["child"], "parent_session_id": ""}))
        subagent = {**self.callbacks["tool"], "platform": "subagent", "parent_session_id": "session-Ω"}
        for callback in ("post_tool_call", "on_session_start"):
            self.assertIsNone(self.request(callback, subagent))
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
            reader = self.refresh()
            self.assertEqual(reader.snapshot()["timeout_seconds"], expected)
        for config in (self.failed(self.config), {"plugins":{"hook_callback_timeout":float("nan")}}, {"plugins":{"hook_callback_timeout":float("inf")}}, []):
            self.config = config
            self.refresh(quality="failed_config_read" if isinstance(config, self.failed) else "unknown")
            self.assertIsNone(self.request())
        self.assertEqual(self.requests(), [])
        self.config = {"plugins":{"hook_callback_timeout":30}}
        reader = self.refresh()
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
        self.request(payload={**self.callbacks["qualified"],"turn_id":"turn-2"})
        self.assertEqual(self.requests()[-1]["request"]["runtime_identity"], original)
        self.assertEqual(self.version_reads, 1)
        self.assertEqual(self.reader().snapshot()["provenance"], "official_effective_config_observation")
        self.assertGreaterEqual(self.reader().snapshot()["age_seconds"], 0)
        self.assertEqual(self.reader().identity_provenance, "startup_captured_identity")

    def test_read_exception_is_private_and_no_implicit_default(self):
        self.read_error = RuntimeError("PRIVATE CONFIG ERROR")
        self.load(wait=False)
        self.await_completion(self.reader(), quality="read_error")
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

    def test_pre_guard_paused_callback_cannot_launch_after_close_or_successor(self):
        self.load()
        predecessor_context = self.ctx
        predecessor = self.reader()
        bridge = predecessor.bridge
        generation = predecessor.generation
        entered, release = threading.Event(), threading.Event()
        original_guard = bridge.child_lock
        class PausedGuard:
            def acquire(self, *args, **kwargs):
                # Only the registered callback pauses, after envelope encoding.
                if threading.current_thread() is worker:
                    entered.set()
                    if not release.wait(2):
                        raise AssertionError("paused callback was not released")
                return original_guard.acquire(*args, **kwargs)
            def release(self):
                original_guard.release()
        bridge.child_lock = PausedGuard()
        outcome = []
        worker = threading.Thread(target=lambda: outcome.append(
            predecessor_context.hooks["pre_llm_call"](**self.callbacks["qualified"])))
        worker.start()
        try:
            self.assertTrue(entered.wait(1), "callback did not reach pre-guard barrier")
            self.assertEqual(bridge.sequence, 1)
            self.assertEqual(predecessor.generation, generation)
            self.assertEqual(self.requests(), [])
            self.assertEqual(self.children, [])
            predecessor_context.unloads[0]()
            predecessor.thread.join(2)
            self.assertFalse(predecessor.thread.is_alive())
            self.assertTrue(bridge.closed)
            self.assertEqual(predecessor.generation, generation + 1)
            self.load()
            successor = self.reader()
            self.assertIsNot(successor, predecessor)
            self.assertIs(sys.modules[SLOT].reader, successor)
            self.assertNotEqual(successor.bridge.nonce, bridge.nonce)
            # Hold the actual successor process alive through predecessor release.
            (self.state / "mode").write_text("lifetime_hold")
            successor_outcome = []
            successor_worker = threading.Thread(target=lambda: successor_outcome.append(self.request()))
            successor_worker.start()
            try:
                limit = time.monotonic() + 1
                while not self.requests() and time.monotonic() < limit:
                    time.sleep(.002)
                self.assertEqual(len(self.requests()), 1)
                successor_pid = self.children[0].pid
                self.assertIsNone(self.children[0].poll())
                release.set()
                limit = time.monotonic() + .5
                while worker.is_alive() and len(self.children) == 1 and time.monotonic() < limit:
                    time.sleep(.002)
                self.assertEqual([child.fixture_live_pids for child in self.children], [[]],
                                 "closed predecessor launched with a live successor")
                worker.join(1)
                self.assertFalse(worker.is_alive())
                self.assertEqual(outcome, [None])
                self.assertEqual(len(self.children), 1,
                                 "closed predecessor launched a child after successor reservation")
                self.assertEqual(self.children[0].pid, successor_pid)
                self.assertEqual(len(self.requests()), 1)
                self.assertEqual(self.requests()[0]["request"]["observation_order"]["process_nonce"],
                                 successor.bridge.nonce)
                (self.state / "release_child").touch()
                successor_worker.join(2)
                self.assertFalse(successor_worker.is_alive())
                self.assertEqual(successor_outcome, [{"context": "bounded cooperative context"}])
            finally:
                release.set()
                (self.state / "release_child").touch()
                successor_worker.join(2)
        finally:
            release.set()
            worker.join(2)
            bridge.child_lock = original_guard
            predecessor_context.unloads[0]()
            predecessor.thread.join(2)
            self.assertFalse(worker.is_alive(), "paused callback leaked")

    def test_held_child_guard_refuses_successor_until_reaped_then_admits(self):
        self.load()
        predecessor = self.reader()
        original = self.module.subprocess.Popen
        started, release = threading.Event(), threading.Event()
        class HeldChild(original):
            def wait(child, *args, **kwargs):
                if threading.current_thread() is worker:
                    started.set()
                    if not release.wait(2):
                        raise AssertionError("owned child reaping barrier was not released")
                return super().wait(*args, **kwargs)
        outcome = []
        worker = threading.Thread(target=lambda: outcome.append(self.request()))
        try:
            with patch.object(self.module.subprocess, "Popen", HeldChild):
                worker.start()
                self.assertTrue(started.wait(1))
                self.assertEqual(len(self.children), 1)
                self.ctx.unloads[0]()
                predecessor.thread.join(2)
                self.assertFalse(predecessor.thread.is_alive())
                self.load(wait=False)
                self.assertIs(self.reader(), predecessor)
                self.assertIsNone(self.request())
                self.assertEqual(len(self.children), 1)
                release.set()
                worker.join(2)
                self.assertFalse(worker.is_alive())
                self.assertEqual(outcome, [None])
                self.assertIsNotNone(self.children[0].returncode)
                self.load()
                self.assertIsNot(self.reader(), predecessor)
                self.assertEqual(self.request(), {"context": "bounded cooperative context"})
                self.assertEqual(len(self.children), 2)
        finally:
            release.set()
            if worker.ident is not None:
                worker.join(2)
            self.assertFalse(worker.is_alive(), "owned-child callback leaked")

    def test_in_progress_spawn_keeps_close_pending_and_reaps_late_child(self):
        self.load()
        predecessor = self.reader()
        bridge = predecessor.bridge
        context = self.ctx
        generation = predecessor.generation
        original = self.module.subprocess.Popen
        entered, release = threading.Event(), threading.Event()
        class PausedSpawn(original):
            def __init__(child, *args, **kwargs):
                entered.set()
                if not release.wait(2):
                    raise AssertionError("owned spawn barrier was not released")
                super().__init__(*args, **kwargs)
        outcome = []
        worker = threading.Thread(target=lambda: outcome.append(
            context.hooks["pre_llm_call"](**self.callbacks["qualified"])))
        try:
            with patch.object(self.module.subprocess, "Popen", PausedSpawn):
                worker.start()
                self.assertTrue(entered.wait(1))
                started = time.monotonic()
                self.assertFalse(context.unloads[0](), "unresolved launch cannot complete close")
                self.assertLess(time.monotonic() - started, .2)
                predecessor.thread.join(2)
                self.assertFalse(predecessor.thread.is_alive())
                self.assertTrue(bridge.closing)
                self.assertFalse(bridge.closed)
                self.assertEqual(predecessor.generation, generation + 1)
                self.assertEqual(self.children, [])
                self.load(wait=False)
                self.assertIs(self.reader(), predecessor)
                self.assertIsNone(self.ctx.hooks["pre_llm_call"].__self__.reader)
                self.assertIsNone(self.request())
                self.assertEqual(self.requests(), [])
                release.set()
                worker.join(2)
                self.assertFalse(worker.is_alive())
                self.assertEqual(outcome, [None])
                self.assertEqual(len(self.children), 1, "late owned spawn was not exercised")
                self.assertIsNotNone(self.children[0].returncode, "late owned spawn was not reaped")
                self.assertEqual(self.requests(), [])
                self.assertTrue(bridge.closed, "cleanup did not complete pending close")
                self.assertTrue(context.unloads[0]())
                self.assertEqual(predecessor.generation, generation + 1)
            self.load()
            self.assertIsNot(self.reader(), predecessor)
            self.assertEqual(self.request(), {"context": "bounded cooperative context"})
            self.assertEqual(len(self.children), 2)
        finally:
            release.set()
            if worker.ident is not None:
                worker.join(2)
            context.unloads[0]()
            predecessor.thread.join(2)
            self.assertFalse(worker.is_alive(), "in-progress launch callback leaked")

    def test_module_eviction_and_concurrent_registration_keep_single_reservation(self):
        self.load()
        predecessor = self.reader()
        module = self.module
        # Native loader eviction cannot remove the separate owned slot namespace.
        with patch.dict(sys.modules, {"synthetic_owned_plugin": module}):
            sys.modules.pop("synthetic_owned_plugin")
            self.load(wait=False)
            self.assertIs(self.reader(), predecessor)
            self.assertIsNone(self.ctx.hooks["pre_llm_call"].__self__.reader)
            self.assertIsNone(self.request())
        self.assertTrue(predecessor.bridge.close())
        predecessor.thread.join(2)
        self.assertFalse(predecessor.thread.is_alive())
        contexts = [self.context_type(), self.context_type()]
        self.contexts.extend(contexts)
        start = threading.Barrier(3)
        workers = []
        def register(context):
            token = PROFILE.set(str(self.home))
            try:
                start.wait(timeout=2)
                module.register(context)
            finally:
                PROFILE.reset(token)
        try:
            for context in contexts:
                worker = threading.Thread(target=register, args=(context,))
                workers.append(worker)
                worker.start()
            start.wait(timeout=2)
            for worker in workers:
                worker.join(2)
                self.assertFalse(worker.is_alive())
            bridges = [context.hooks["pre_llm_call"].__self__ for context in contexts]
            admitted = [bridge for bridge in bridges if bridge.reader is not None]
            self.assertEqual(len(admitted), 1, "concurrent registration created multiple readers")
            successor = admitted[0].reader
            self.assertIs(sys.modules[SLOT].reader, successor)
            self.assertEqual(len(self.readers), 2)
            self.await_completion(successor)
            self.assertEqual(admitted[0].pre_llm_call(**self.callbacks["qualified"]),
                             {"context": "bounded cooperative context"})
            self.assertEqual(len(self.children), 1)
        finally:
            start.abort()
            for worker in workers:
                worker.join(2)
                self.assertFalse(worker.is_alive(), "registration worker leaked")

    def test_registration_error_closes_its_reservation_before_replacement(self):
        self.load()
        predecessor = self.reader()
        self.assertTrue(predecessor.bridge.close())
        predecessor.thread.join(2)
        module = self.module
        context = self.context_type()
        self.contexts.append(context)
        def reject_unload(callback):
            raise RuntimeError("synthetic registration failure")
        context.on_unload = reject_unload
        token = PROFILE.set(str(self.home))
        try:
            self.assertIsNone(module.register(context))
        finally:
            PROFILE.reset(token)
        failed = self.reader()
        self.assertIsNot(failed, predecessor)
        failed.thread.join(2)
        self.assertFalse(failed.thread.is_alive())
        self.assertTrue(failed.bridge.closed)
        self.assertEqual(set(context.hooks), {"pre_tool_call"})
        self.assertEqual(self.children, [])
        self.load()
        self.assertIsNot(self.reader(), failed)
        self.assertEqual(self.request(), {"context": "bounded cooperative context"})
        self.assertEqual(len(self.children), 1)

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
            self.assertEqual(set(self.ctx.hooks), {"pre_tool_call"})
            self.assertEqual(self.reads, [])
        (self.asset / "bridge_config.json").write_text(" " * 65537)
        self.load(wait=False)
        self.assertEqual(set(self.ctx.hooks), {"pre_tool_call"})
        self.assertEqual(self.reads, [])
        self.assertEqual(self.requests(), [])

    def test_changed_native_profile_during_refresh_cannot_deliver_under_startup_identity(self):
        self.load()
        self.rotate_profile = True
        self.refresh(quality="profile_unavailable")
        self.assertIsNone(self.request())
        self.assertEqual(self.requests(), [])

    def test_incomplete_git_startup_descriptor_cannot_deliver_context(self):
        for field in ("commit", "distance"):
            self.version = types.SimpleNamespace(**{**VERSION,field:None})
            self.load(wait=False)
            self.await_completion(self.reader(), kind="capture", quality="identity_unavailable")
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
            self.await_completion(reader, kind="capture", quality="identity_unavailable")
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
        self.assertEqual(set(self.ctx.hooks), {"pre_tool_call"})
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
        with self.reader().lock:
            iteration = self.reader().lock.counts["timeout"] + 1
        self.await_completion(self.reader(), iteration=iteration, quality="failed_config_read")
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
        self.await_completion(self.reader(), kind="capture", quality="identity_unavailable")
        self.assertIsNone(self.request())
        self.assertEqual(self.requests(), [])
        self.assertIsNone(self.reader().identity)

    def test_completion_helper_rejects_pending_target_and_previous_snapshot(self):
        self.load()
        reader = self.reader()
        self.block_read = True
        with reader.lock:
            iteration = reader.lock.counts["timeout"] + 1
            reader.next_refresh = 0
        reader.wake.set()
        try:
            self.await_entry(reader, iteration=iteration)
            with reader.lock:
                self.assertTrue(reader.pending)
                self.assertIn(("timeout", iteration - 1), reader.lock.completed)
            with self.assertRaisesRegex(AssertionError, "did not publish completion"):
                self.await_completion(reader, iteration=iteration, timeout=.05)
            self.assertIsNone(self.request())
            self.assertEqual(self.requests(), [])
            self.assertFalse((self.state / "pid").exists())
        finally:
            self.release.set()
        self.await_completion(reader, iteration=iteration)

    def completed_clock_read(self, finished):
        self.clock = 100.0
        self.block_read = True
        self.load(wait=False)
        reader = self.reader()
        try:
            self.await_entry(reader)
            self.assertTrue(self.entered.wait(2))
            with reader.lock:
                self.assertTrue(reader.pending)
                self.assertIsNotNone(reader.identity)
            self.assertEqual(self.reads, [str(self.home)])
            self.clock = finished
        finally:
            self.release.set()
        result = self.await_completion(reader)
        self.assertEqual(result[1], 30.0)
        with reader.lock:
            self.assertFalse(reader.pending)
            self.assertEqual(reader.quality, "ok")
            self.assertEqual(reader.observed_timeout, 30.0)
        # Callback deadlines and child teardown continue on elapsed real time,
        # independently of the explicitly controlled provider clock above.
        anchor = time.monotonic()
        self.module.time.monotonic = lambda: finished + time.monotonic() - anchor
        return reader

    def test_completed_slow_timeout_read_keeps_entry_age_and_starts_no_child(self):
        reader = self.completed_clock_read(106.0)
        # Actual registered callback is the stale-observation consumer.
        self.assertIsNone(self.request())
        self.assertEqual(self.requests(), [])
        self.assertFalse((self.state / "pid").exists(), "stale read started owned child")
        self.assertIsNone(reader.snapshot())
        self.assertEqual(reader.observed_at, 100.0)

    def test_completed_fast_timeout_read_delivers_through_registered_callback(self):
        reader = self.completed_clock_read(100.1)
        self.assertEqual(self.request(), {"context": "bounded cooperative context"})
        self.assertEqual(len(self.requests()), 1)
        self.assertTrue((self.state / "pid").exists(), "fast read did not start owned child")
        self.assertNotIn("PRIVATE", json.dumps(self.requests()))
        self.assertIsNotNone(reader.snapshot())
        self.assertEqual(reader.observed_at, 100.0)


class PersonGateTests(unittest.TestCase):
    """The pre_tool_call gate: Hermes asks the person before an agent's terminal command runs
    the human namespace or a permission-changing command; everything else passes untouched."""

    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location("person_gate_under_test", SOURCE)
        cls.module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.module)

    def decide(self, command, tool="terminal"):
        result = self.module.pre_tool_call(tool_name=tool, args={"command": command}, task_id="t")
        return None if result is None else (result["action"], result["rule_key"])

    def test_gated_commands_ask_the_person(self):
        for command, gated in (
            ("herdr-threads human me init", "human"),
            ("ht human", "human"),
            ("/usr/local/bin/herdr-threads human send t --body x", "human"),
            ("herdr-threads 'human' me init", "human"),
            ("cd /tmp && ht setup claude --json", "setup"),
            ("X=1 herdr-threads unsetup codex", "unsetup"),
            ("herdr-threads doctor fix", "doctor fix"),
            ("echo hi; ht internal installer-integrations x", "internal installer-integrations"),
            ("ht inbox|herdr-threads human retry r", "human"),
        ):
            self.assertEqual(self.decide(command), ("approve", f"herdr-threads:{gated}"), command)

    def test_ordinary_and_unrelated_commands_pass(self):
        for command in ("herdr-threads inbox", "ht send t --body 'human setup'", "herdr-threads doctor",
                        "herdr-threads doctor --debug", "echo herdr-threads human", "ls human",
                        "herdr-threadsx human", "herdr-threads --json inbox"):
            self.assertIsNone(self.decide(command), command)
        self.assertIsNone(self.decide("herdr-threads human", tool="read_file"))
        self.assertIsNone(self.module.pre_tool_call(tool_name="terminal", args=None))


if __name__ == "__main__":
    unittest.main()
