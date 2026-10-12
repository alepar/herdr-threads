#!/usr/bin/env python3
"""Hook-output mutator for the native gate's downgrade flow (test-infra spec D3).

  step_mutator.py --ctl DIR -- <original hook argv...>

Reads stdin, runs the original hook argv as a child with that stdin, and prints the child's stdout
and exits with its code, byte for byte, unless an armed file exists in DIR and the stdout parses as
``{"injectSteps":[...]}`` holding a ``systemMessage`` step:

  DIR/mutate-armed         mode ``abort``: print the probe-verified unsupported step (a tool call
                           agy rejects, which aborts the turn; probe 19a-abort)
  DIR/mutate-armed-ignore  mode ``ignore``: print a well-formed unknown step key that agy silently
                           ignores, so the frame never reaches the transcript

The armed file is then renamed to ``DIR/mutate-fired`` and the JSON record ``{mode, original_stdout,
frame_ids}`` is written there. It fires once per arming; a stdout with no ``systemMessage`` step does
not consume it. Python 3.9 compatible, standard library only.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys

ARMED = "mutate-armed"
ARMED_IGNORE = "mutate-armed-ignore"
FIRED = "mutate-fired"
MODES = (("abort", ARMED), ("ignore", ARMED_IGNORE))

# D3: probe-verified (probe-toolcall/runs/19a-abort/hooks.jsonl); agy aborts the turn on it.
ABORT_OUTPUT = b'{"injectSteps":[{"toolCall":{"name":"run_command","args":{"CommandLine":"true"}}}]}'
# A well-formed step with an unknown key: agy ignores it without a Stop error (spec D3 roast variant).
IGNORE_OUTPUT = b'{"injectSteps":[{"htIgnoredStep":{"text":"herdr-threads gate: ignored step"}}]}'

_NAME = r'(?: "(?:[^"\\]|\\.)*")?'
HEADER = re.compile(r"^\[herdr-threads\] (\S+) (\S+) in (\S+)" + _NAME + r" from (\S+)" + _NAME + r"(.*):$", re.M)


def has_system_message_step(stdout_bytes):
    """True when the bytes parse as ``{"injectSteps":[...]}`` with a ``systemMessage`` step."""
    try:
        doc = json.loads(stdout_bytes.decode("utf-8"))
    except (UnicodeDecodeError, ValueError):
        return False
    steps = doc.get("injectSteps") if isinstance(doc, dict) else None
    return isinstance(steps, list) and any(isinstance(s, dict) and "systemMessage" in s for s in steps)


def frame_ids(stdout_bytes):
    """Message ids of the ``[herdr-threads] <kind> <id> in <thread> from <sender>`` headers, in order."""
    try:
        doc = json.loads(stdout_bytes.decode("utf-8"))
    except (UnicodeDecodeError, ValueError):
        return []
    ids = []
    steps = doc.get("injectSteps") if isinstance(doc, dict) else None
    for step in steps if isinstance(steps, list) else []:
        message = step.get("systemMessage") if isinstance(step, dict) else None
        text = message.get("systemMessage") if isinstance(message, dict) else message
        if isinstance(text, str):
            ids.extend(m.group(2) for m in HEADER.finditer(text))
    return ids


def mutate(stdout_bytes, armed_mode):
    """The bytes to print: the mode's broken step for a ``systemMessage`` stdout, else the input as is."""
    if armed_mode not in ("abort", "ignore") or not has_system_message_step(stdout_bytes):
        return stdout_bytes
    return ABORT_OUTPUT if armed_mode == "abort" else IGNORE_OUTPUT


def claim_arming(ctl):
    """Rename the armed file to ``mutate-fired``; the mode, or None when nothing is armed (or lost the race)."""
    for mode, name in MODES:
        armed = os.path.join(ctl, name)
        if os.path.exists(armed):
            try:
                os.rename(armed, os.path.join(ctl, FIRED))
            except OSError:
                return None
            return mode
    return None


def armed_mode(ctl):
    for mode, name in MODES:
        if os.path.exists(os.path.join(ctl, name)):
            return mode
    return None


def parent_image(ppid=None):
    """The executable path of the parent process (agy for a hook), or None."""
    ppid = os.getppid() if ppid is None else ppid
    try:
        return os.readlink("/proc/%d/exe" % ppid)
    except OSError:
        pass
    try:  # macOS: proc_pidpath (no exec, so it also works inside the gate's sandbox profile)
        import ctypes
        libproc = ctypes.CDLL("/usr/lib/libproc.dylib")
        buffer = ctypes.create_string_buffer(4096)
        if libproc.proc_pidpath(int(ppid), buffer, len(buffer)) > 0:
            return buffer.value.decode("utf-8", "replace")
    except (OSError, AttributeError, ValueError):
        pass
    try:
        out = subprocess.run(["/bin/ps", "-p", str(ppid), "-o", "comm="], stdout=subprocess.PIPE,
                             stderr=subprocess.DEVNULL, universal_newlines=True, timeout=10).stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return None
    return out if out.startswith("/") else None


def child_env(env, image):
    """``env`` with the directory of an agy ``image`` first on ``PATH``; ``env`` itself otherwise."""
    if not image or os.path.basename(image) not in ("agy", "antigravity"):
        return env
    out = dict(env)
    out["PATH"] = os.path.dirname(image) + (os.pathsep + env["PATH"] if env.get("PATH") else "")
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(prog="step_mutator")
    ap.add_argument("--ctl", required=True)
    ap.add_argument("child", nargs=argparse.REMAINDER)
    args = ap.parse_args(argv)
    child = args.child[1:] if args.child[:1] == ["--"] else args.child
    if not child:
        sys.stderr.write("step_mutator: no hook argv\n")
        return 2
    stdin_bytes = sys.stdin.buffer.read()
    image = parent_image()
    env = child_env(os.environ, image)
    result = subprocess.run(child, input=stdin_bytes, stdout=subprocess.PIPE, env=env)
    out = result.stdout
    if armed_mode(args.ctl) and has_system_message_step(out):
        mode = claim_arming(args.ctl)
        if mode is not None:
            record = {"mode": mode, "original_stdout": out.decode("utf-8", "replace"), "frame_ids": frame_ids(out),
                      "parent_image": image, "child_path": env.get("PATH")}
            with open(os.path.join(args.ctl, FIRED), "w") as handle:
                json.dump(record, handle)
            out = mutate(out, mode)
    sys.stdout.buffer.write(out)
    sys.stdout.buffer.flush()
    return result.returncode


if __name__ == "__main__":
    sys.exit(main())
