"""Owned stdlib Hermes callback bridge; offered context is not native/model delivery.

Only initialized, already-loaded native APIs are observed. ONE refresh thread
holds the startup identity and registration contextvars for its lifetime. Its
native reads are uncancellable, outside callbacks, and may hang. No replacement
is started while that thread lives, even across native plugin module eviction.
Callbacks read a nonblocking snapshot: pending/failure/unknown/age>5s skips Rust.
Refreshes are at least 1s after completion. Unload invalidates publication and
signals stop, joining for at most 50ms; it cannot claim a blocked native read
stopped. Neither config observations nor source changes refresh startup identity.
"""
from collections import OrderedDict
import contextvars
import hashlib
import json
import math
import os
from pathlib import Path
import selectors
import subprocess
import sys
import threading
import time
import types
import unicodedata
import uuid

HOOKS = ("pre_llm_call", "post_tool_call", "on_session_start", "on_session_reset")
IDS = ("platform", "session_id", "parent_session_id", "task_id", "turn_id", "tool_call_id", "api_request_id")
SLOT = "_herdr_threads_hermes_reader_schema1"
CHILD_RESTRICTION = "Subagents may discover, read and summarize; never check in for this seat, accept or ACK. Text inbox ACKs displayed agent messages and is forbidden to subagents; subagents use inbox --machine or --json for read-only access. Every herdr-threads write (any accept, ack, check-in, send, leave, invite or other mutation) acts as the top-level seat and is forbidden to subagents. Return message IDs and summaries to the top-level agent."
# First 16 SHA-256 hex chars of sorted compact JSON of the normalized schema
# declaration in testdata/callbacks.json. This identifies transport, not the
# downstream domain descriptor/qualification or native measurement.
BRIDGE_CONTRACT = "d73f44f51c4ef9dd"


def opaque(value, empty=False, limit=256):
    return (isinstance(value, str) and (empty or bool(value))
            and len(value.encode("utf-8")) <= limit
            and not any(unicodedata.category(c) == "Cc" for c in value))


def strict_json(raw):
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError("duplicate field")
            result[key] = value
        return result
    def constant(_):
        raise ValueError("nonfinite JSON")
    return json.loads(raw, object_pairs_hook=pairs, parse_constant=constant)


def origin(module):
    file = getattr(module, "__file__", None)
    spec = getattr(getattr(module, "__spec__", None), "origin", None)
    if not file or not spec or not os.path.isabs(file) or not os.path.isabs(spec):
        raise ValueError("origin_unavailable")
    resolved = Path(file).resolve(strict=True)
    if resolved != Path(spec).resolve(strict=True):
        raise ValueError("origin_mismatch")
    return resolved


class NativeProvider:
    """Observe APIs in the native initialization context, never bootstrap or import.

    API/module references captured before the worker starts. Profile/home and
    origins are observed there; no global HOME/HERMES_HOME or sys.path mutation.
    No plugin discovery, auth/model accessor or prelaunch diagnostic is used.
    """
    def __init__(self, ctx, asset):
        self.asset = asset
        self.captured_home = None
        self.modules = {name: sys.modules.get(name) for name in (
            "hermes_bootstrap", "hermes_constants", "hermes_cli.profiles",
            "hermes_cli.version_info", "hermes_cli.config", "hermes_cli.plugins")}
        plugins = self.modules["hermes_cli.plugins"]
        if plugins is None or not isinstance(ctx, getattr(plugins, "PluginContext", ())):
            raise ValueError("native_context_unavailable")
        self.version_api = getattr(self.modules["hermes_cli.version_info"], "get_version_info", None)
        self.profile_api = getattr(self.modules["hermes_cli.profiles"], "current_profile_name", None)
        self.resolve_profile_api = getattr(self.modules["hermes_cli.profiles"], "resolve_profile_env", None)
        self.home_api = getattr(self.modules["hermes_constants"], "get_hermes_home", None)
        config_module = self.modules["hermes_cli.config"]
        self.config_api = getattr(config_module, "load_config_readonly", None)
        self.failed_type = getattr(config_module, "FailedConfigRead", None)
        if (not all(callable(api) for api in (self.version_api, self.profile_api,
                                             self.resolve_profile_api, self.home_api, self.config_api))
                or not isinstance(self.failed_type, type)):
            raise ValueError("native_api_unavailable")

    def capture(self):
        root = origin(self.modules["hermes_bootstrap"]).parent
        origins = {name: str(origin(module)) for name, module in self.modules.items()}
        for name, path in origins.items():
            expected_parent = root / "hermes_cli" if name.startswith("hermes_cli.") else root
            if Path(path).parent != expected_parent:
                raise ValueError("origin_mismatch")
        if not os.path.isabs(sys.executable):
            raise ValueError("interpreter_unavailable")
        interpreter = str(Path(sys.executable).resolve(strict=True))
        home = self.home_api()
        lexical_home = str(home)
        profile = self.profile_api(default=None)
        if not opaque(profile) or profile == "custom":
            raise ValueError("profile_unavailable")
        resolved_home = self.resolve_profile_api(profile)
        if (not os.path.isabs(lexical_home) or lexical_home != str(resolved_home)
                or Path(home).resolve(strict=True) != self.asset.parent.parent.resolve(strict=True)):
            raise ValueError("profile_mismatch")
        self.captured_home = lexical_home
        info = self.version_api()
        source = getattr(info, "source", None)
        if (not isinstance(source, str) or source not in (
                "build", "commit-build", "ci", "docker", "fallback", "git", "local", "nix")):
            raise ValueError("identity_unavailable")
        descriptor = {"release_version": None, "source": source,
                      "base_version": info.base_version, "derived_version": info.derived_version,
                      "commit": info.commit, "dirty": info.dirty, "distance": info.distance}
        for key in ("base_version", "derived_version"):
            if not opaque(descriptor[key], limit=128):
                raise ValueError("identity_unavailable")
        for key in ("release_version", "commit"):
            value = descriptor[key]
            if value is not None and not opaque(value, limit=128):
                raise ValueError("identity_unavailable")
        commit = descriptor["commit"]
        if source == "git" and (commit is None or descriptor["distance"] is None):
            raise ValueError("identity_unavailable")
        if commit is not None and (len(commit) != 40 or any(c not in "0123456789abcdef" for c in commit)):
            raise ValueError("identity_unavailable")
        if (type(descriptor["dirty"]) is not bool or
                (descriptor["distance"] is not None and
                 (type(descriptor["distance"]) is not int or not 0 <= descriptor["distance"] <= 4294967295))):
            raise ValueError("identity_unavailable")
        canonical = json.dumps({**descriptor, "schema_version": 1}, sort_keys=True,
                               separators=(",", ":"), ensure_ascii=False).encode()
        identity = {"key": "build:" + hashlib.sha256(canonical).hexdigest(), **descriptor}
        return identity, {"interpreter": interpreter, "module_origins": origins,
                          "profile": profile, "lexical_home": lexical_home,
                          "physical_home": str(Path(home).resolve(strict=True))}

    def observe_timeout(self):
        if str(self.home_api()) != self.captured_home:
            return None, "profile_unavailable"
        config = self.config_api()
        if str(self.home_api()) != self.captured_home:
            return None, "profile_unavailable"
        # No raw errors/config leave the API call; fallback provenance is unknown.
        if isinstance(config, self.failed_type):
            return None, "failed_config_read"
        if not isinstance(config, dict):
            return None, "unknown"
        plugins = config.get("plugins")
        raw = plugins.get("hook_callback_timeout") if isinstance(plugins, dict) else None
        if raw is None:
            return 30.0, "ok"
        # Pinned native float/default/clamp semantics, only for a successful read.
        try:
            value = float(raw)
        except (TypeError, ValueError, OverflowError):
            return 30.0, "ok"
        if not math.isfinite(value):
            return None, "unknown"
        if value < 0:
            value = 30.0
        return min(value, 600.0), "ok"


class Reader:
    """Single context-preserving worker; callback snapshots never wait for native I/O."""
    def __init__(self, provider):
        self.provider = provider
        self.lock = threading.Lock()
        self.stop = threading.Event()
        self.wake = threading.Event()
        self.identity = None
        self.identity_provenance = "startup_captured_identity"
        self.startup = None
        self.quality = "unknown"
        self.fallback = None
        self.pending = True
        self.observed_timeout = None
        self.observed_at = 0
        self.next_refresh = 0
        self.valid = True
        self.generation = 1
        native_context = contextvars.copy_context()
        self.thread = threading.Thread(target=native_context.run, args=(self.run,),
                                       name="herdr-threads-hermes-native-reader", daemon=True)

    def run(self):
        try:
            identity, startup = self.provider.capture()
        except Exception:
            with self.lock:
                self.pending = False
                self.quality = "identity_unavailable"
            return
        with self.lock:
            if not self.valid:
                return
            self.identity = identity
            self.startup = startup
        while not self.stop.is_set():
            delay = max(0, self.next_refresh - time.monotonic())
            if delay:
                self.wake.wait(delay)
                self.wake.clear()
                continue
            with self.lock:
                if not self.valid:
                    return
                self.pending = True
                generation = self.generation
            started = time.monotonic()
            try:
                timeout, quality = self.provider.observe_timeout()
            except Exception:
                timeout, quality = None, "read_error"
            finished = time.monotonic()
            with self.lock:
                if not self.valid or generation != self.generation or self.stop.is_set():
                    return
                # A read older than the stale bound cannot become a fresh observation
                # simply because it finally completed; there is no atomic read proof.
                self.observed_at = started
                self.observed_timeout = timeout
                self.quality = quality
                self.pending = False
                self.next_refresh = finished + 1.0

    def snapshot(self):
        if not self.lock.acquire(blocking=False):
            return None
        try:
            age = time.monotonic() - self.observed_at
            if (self.stop.is_set() or not self.valid or self.pending or self.quality != "ok" or self.identity is None
                    or self.observed_timeout is None or age < 0 or age > 5):
                return None
            return {"timeout_seconds": self.observed_timeout,
                    "age_seconds": age, "provenance": "official_effective_config_observation",
                    "identity": self.identity, "generation": self.generation}
        finally:
            self.lock.release()

    def close(self):
        # stop is checked even when publication lock is contended. No unbounded
        # join, forced thread cancellation or replacement of a still-live reader.
        self.stop.set()
        self.wake.set()
        if self.lock.acquire(blocking=False):
            try:
                self.valid = False
                self.generation += 1
            finally:
                self.lock.release()
        if self.thread is not threading.current_thread():
            self.thread.join(.05)


def reserve_reader(provider):
    # An OWNED module namespace survives the native loader evicting its plugin
    # module. No Hermes or stdlib module is modified. Reserve/start under one
    # nonblocking lock so concurrent registers cannot each start a worker.
    candidate = types.ModuleType(SLOT)
    candidate.lock = threading.Lock()
    candidate.reader = None
    slot = sys.modules.setdefault(SLOT, candidate)
    if not slot.lock.acquire(blocking=False):
        return None
    try:
        if slot.reader is not None:
            if slot.reader.thread.is_alive():
                return None
            previous = getattr(slot.reader, "bridge", None)
            if previous is not None:
                if not previous.child_lock.acquire(blocking=False):
                    return None
                try:
                    child = previous.unreaped_child
                    if child is not None and child.poll() is None:
                        return None
                finally:
                    previous.child_lock.release()
        reader = Reader(provider)
        slot.reader = reader
        reader.thread.start()
        return reader
    finally:
        slot.lock.release()


def read_settings(asset):
    with (asset / "bridge_config.json").open("rb") as stream:
        raw = stream.read(65537)
    if len(raw) > 65536:
        raise ValueError("config_too_large")
    settings = strict_json(raw)
    if (not isinstance(settings, dict) or set(settings) != {
            "schema_version", "rust_executable", "state_root", "host_endpoint",
            "bridge_schema_version", "installation_token"}
            or type(settings["schema_version"]) is not int or settings["schema_version"] != 1
            or type(settings["bridge_schema_version"]) is not int or settings["bridge_schema_version"] != 1
            or not opaque(settings["installation_token"])):
        raise ValueError("config_unavailable")
    for key in ("rust_executable", "state_root", "host_endpoint"):
        if not opaque(settings[key], limit=4096) or not os.path.isabs(settings[key]):
            raise ValueError("config_unavailable")
    return settings


def shape(kwargs):
    def kind(value):
        if value is None: return "null"
        if isinstance(value, str): return "string"
        if isinstance(value, bool): return "boolean"
        if isinstance(value, (int, float)): return "number"
        if isinstance(value, dict): return "object"
        if isinstance(value, (list, tuple)): return "array"
        return "other"
    return {key: {"presence": "present" if key in kwargs else "missing",
                  "type": kind(kwargs[key]) if key in kwargs else "absent"} for key in IDS}


class Bridge:
    """Metadata/role projection only; Rust owns durable lifecycle and all authority."""
    def __init__(self, settings, reader):
        self.settings = settings
        self.reader = reader
        self.role_lock = threading.Lock()
        self.roles = OrderedDict()
        self.entry_lock = threading.Lock()
        self.child_lock = threading.Lock()
        self.sequence = 0
        self.nonce = str(uuid.uuid4())
        self.closed = False
        self.unreaped_child = None

    def close(self):
        self.closed = True
        if self.reader is not None:
            self.reader.close()

    def remember_role(self, session, turn, role, now):
        if not self.role_lock.acquire(blocking=False):
            return False
        try:
            # At most 128 entries; purge is bounded and atomic with insertion.
            for key, (_, last) in list(self.roles.items()):
                if now - last > 600:
                    del self.roles[key]
            key = (session, turn)
            self.roles[key] = (role, now)
            self.roles.move_to_end(key)
            while len(self.roles) > 128:
                self.roles.popitem(last=False)
            return True
        finally:
            self.role_lock.release()

    def associated(self, session, turn, now):
        if not self.role_lock.acquire(blocking=False):
            return False
        try:
            key = (session, turn)
            hit = self.roles.get(key)
            if hit is None or hit[0] != "top" or now - hit[1] > 600:
                self.roles.pop(key, None)
                return False
            self.roles[key] = ("top", now)
            self.roles.move_to_end(key)
            return True
        finally:
            self.role_lock.release()

    def reset_roles(self, session):
        if not self.role_lock.acquire(blocking=False):
            return False
        try:
            for key in list(self.roles):
                if key[0] == session:
                    del self.roles[key]
            return True
        finally:
            self.role_lock.release()

    def pre_llm_call(self, **kwargs):
        return self.callback("pre_llm_call", kwargs)

    def post_tool_call(self, **kwargs):
        return self.callback("post_tool_call", kwargs)

    def on_session_start(self, **kwargs):
        return self.callback("on_session_start", kwargs)

    def on_session_reset(self, **kwargs):
        return self.callback("on_session_reset", kwargs)

    def callback(self, name, kwargs):
        try:
            return self._callback(name, kwargs)
        except Exception:
            return None  # no exception body/config/user/tool content is logged

    def _callback(self, name, kwargs):
        entered = time.monotonic()
        started_at = int(time.time() * 1000)
        if not self.entry_lock.acquire(blocking=False):
            return None
        try:
            self.sequence += 1
            sequence = self.sequence
        finally:
            self.entry_lock.release()
        if self.closed or self.reader is None:
            return None
        # Validate provided identifiers before encoding anything. No truncation.
        for key in IDS:
            if key in kwargs and kwargs[key] is not None and not opaque(kwargs[key], empty=key in ("parent_session_id", "platform")):
                return None
        session, turn = kwargs.get("session_id"), kwargs.get("turn_id")
        if not opaque(session):
            return None
        platform = kwargs.get("platform")
        if platform not in (None, "", "cli"):
            return None
        parent = kwargs.get("parent_session_id")
        # Reset association clearing remains independent of delivery/reader state.
        if name == "on_session_reset":
            if kwargs.get("reason") != "new_session" or not self.reset_roles(session):
                return None
        observation = self.reader.snapshot()
        if observation is None:
            return None
        if name == "pre_llm_call":
            if not opaque(parent, empty=True) or not opaque(turn):
                return None
            role = "child" if parent else "top"
            if not self.remember_role(session, turn, role, entered):
                return None
        elif name == "post_tool_call":
            if not opaque(turn) or not self.associated(session, turn, entered):
                return None
        timeout = observation["timeout_seconds"]
        lifecycle = name in ("on_session_start", "on_session_reset")
        local_cap = 4.5 if lifecycle else 1.2
        class_cap = 5.0 if lifecycle else 1.5
        if timeout > 0 and timeout <= .2:
            return None
        span = min(local_cap, class_cap, timeout - .1 if timeout else local_cap)
        deadline = entered + span
        if time.monotonic() >= deadline:
            return None
        if name == "pre_llm_call" and parent:
            return {"context": CHILD_RESTRICTION}
        # Exact native session/turn key is stable across callback retries and
        # process restarts. Observers/missing turns have invocation-only IDs.
        if turn:
            identity = json.dumps([name, session, turn, kwargs.get("tool_call_id"),
                                   kwargs.get("api_request_id")], ensure_ascii=False, separators=(",", ":"))
            event = "hermes:" + hashlib.sha256(identity.encode()).hexdigest()
        else:
            event = "hermes:" + self.nonce + ":" + str(sequence)
        budget_ms = int(span * 1000)
        envelope = {"schema_version": 1, "callback": name,
                    **{key: kwargs.get(key) for key in IDS}, "event_id": event,
                    "observation_order": {"process_nonce": self.nonce, "sequence": sequence,
                                          "observed_at_millis": started_at, "callback_budget_millis": budget_ms},
                    "started_at": started_at, "deadline_at": started_at + budget_ms,
                    "reset_reason": "new_session" if name == "on_session_reset" else None,
                    "runtime_identity": observation["identity"], "identity_unavailable_reason": None,
                    "shape": shape(kwargs), "bridge_schema_contract_id": BRIDGE_CONTRACT}
        encoded = json.dumps(envelope, ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode()
        if len(encoded) > 65536 or not self.child_lock.acquire(blocking=False):
            return None
        try:
            output = self.child(encoded, deadline)
            # Every callback entry invalidates older work, including contenders
            # skipped while the child lock is held. No late output is published.
            if self.closed or self.sequence != sequence or time.monotonic() >= deadline:
                return None
            if self.reader.snapshot() is None:
                return None
            result = strict_json(output)
            if not isinstance(result, dict) or set(result) != {"context", "lifecycle_ack"}:
                return None
            context, ack = result["context"], result["lifecycle_ack"]
            if context is not None and (not isinstance(context, str) or len(context.encode()) > 4096):
                return None
            if ack is not None:
                if (not isinstance(ack, dict) or set(ack) != {"event_id", "session_id", "mode"}
                        or ack["event_id"] != event or ack["session_id"] != session
                        or ack["mode"] not in ("startup", "current", "clear")):
                    return None
            if name == "pre_llm_call" and context and ack is not None:
                if not self.entry_lock.acquire(blocking=False):
                    return None
                try:
                    fresh = self.reader.snapshot()
                    if (self.closed or self.sequence != sequence or time.monotonic() >= deadline
                            or fresh is None or fresh["generation"] != observation["generation"]):
                        return None
                    return {"context": context}
                finally:
                    self.entry_lock.release()
            return None
        finally:
            self.child_lock.release()

    def child(self, encoded, deadline):
        if self.unreaped_child is not None:
            if self.unreaped_child.poll() is None:
                raise TimeoutError  # no successor until the owned child is reaped
            self.unreaped_child = None
        argv = [self.settings["rust_executable"], "--state-dir", self.settings["state_root"],
                "--host-endpoint", self.settings["host_endpoint"], "hook", "hermes"]
        # Reserve cleanup time inside the SAME decreasing callback deadline.
        io_deadline = deadline - .06
        if time.monotonic() >= io_deadline:
            raise TimeoutError
        selector = selectors.DefaultSelector()
        try:
            process = subprocess.Popen(argv, shell=False, stdin=subprocess.PIPE,
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, close_fds=True)
        except Exception:
            selector.close()
            raise
        output = bytearray()
        stderr_count = 0
        written = 0
        try:
            for stream in (process.stdin, process.stdout, process.stderr):
                os.set_blocking(stream.fileno(), False)
            selector.register(process.stdin, selectors.EVENT_WRITE, "stdin")
            selector.register(process.stdout, selectors.EVENT_READ, "stdout")
            selector.register(process.stderr, selectors.EVENT_READ, "stderr")
            while selector.get_map():
                remaining = io_deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError
                for key, _ in selector.select(remaining):
                    stream = key.fileobj
                    try:
                        if key.data == "stdin":
                            written += os.write(stream.fileno(), encoded[written:written + 4096])
                            if written == len(encoded):
                                selector.unregister(stream)
                                stream.close()
                        else:
                            chunk = os.read(stream.fileno(), 4096)
                            if not chunk:
                                selector.unregister(stream)
                                stream.close()
                            elif key.data == "stdout":
                                output.extend(chunk)
                                if len(output) > 8192:
                                    raise ValueError("output_too_large")
                            else:
                                stderr_count += len(chunk)
                                if stderr_count > 8192:
                                    raise ValueError("stderr_too_large")
                    except BlockingIOError:
                        continue
            remaining = io_deadline - time.monotonic()
            if remaining <= 0 or process.wait(timeout=remaining) != 0:
                raise ValueError("child_failed")
            return bytes(output)
        finally:
            selector.close()
            for stream in (process.stdin, process.stdout, process.stderr):
                stream.close()
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=min(.02, max(0, deadline - time.monotonic())))
                except subprocess.TimeoutExpired:
                    process.kill()
                    # Owned immediate child only; never shared servers/panes.
                    # If OS reaping itself exceeds this budget, keep ownership
                    # and refuse successors; do not queue background cleanup.
                    try:
                        process.wait(timeout=max(.001, deadline - time.monotonic()))
                    except subprocess.TimeoutExpired:
                        self.unreaped_child = process
                        raise


def register(ctx):
    """Register exactly four fail-open/cooperative hooks in the native context."""
    asset = Path(__file__).parent
    reader = None
    try:
        settings = read_settings(asset)
        provider = NativeProvider(ctx, asset)
        reader = reserve_reader(provider)
        bridge = Bridge(settings, reader)
        if reader is not None:
            reader.bridge = bridge
        # Native unload participates in lifetime invalidation; if no owned slot
        # was available, these hooks remain inert and never start a successor.
        ctx.on_unload(bridge.close)
        for name in HOOKS:
            ctx.register_hook(name, getattr(bridge, name))
    except Exception:
        if reader is not None:
            reader.close()
        return None
