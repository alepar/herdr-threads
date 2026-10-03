#!/usr/bin/env python3
"""Tier-1 capture hook (nested spec §D6): `capture_hook.py <dir> <nonce-file>`.

Reads one hook payload on stdin, writes `{event, stdin, stdout}` to a new file in <dir> (the §D5
`capture/tier1/*.json` contract) and prints the nonce envelope for the event:
`{"hookSpecificOutput":{"hookEventName":<event>,"additionalContext":"canary-nonce-<event>: <N>"}}`.
<nonce-file> is JSON `{"SessionStart": "<nonce>", "PreToolUse": "<nonce>"}`, written once per probe.
An event without a nonce is captured and answered with empty stdout. Pure stdlib; always exits 0 once the
capture is written so a failing canary never blocks the model run it observes."""
import json, os, sys, time


def event_of(raw):
    try:
        doc = json.loads(raw)
    except ValueError:
        return "unknown"
    name = doc.get("hook_event_name") if isinstance(doc, dict) else None
    return name if isinstance(name, str) and name.isalnum() else "unknown"


def envelope(event, nonces):
    nonce = nonces.get(event)
    if not isinstance(nonce, str) or not nonce:
        return ""
    return json.dumps({"hookSpecificOutput": {"hookEventName": event,
                                              "additionalContext": f"canary-nonce-{event}: {nonce}"}},
                      separators=(",", ":"))


def capture(directory, nonce_file, raw):
    """Write the capture record and return the stdout string for the harness."""
    with open(nonce_file, encoding="utf-8") as f:
        nonces = json.load(f)
    event = event_of(raw)
    out = envelope(event, nonces)
    os.makedirs(directory, exist_ok=True)
    while True:  # name sorts by capture time; O_EXCL keeps concurrent hooks from overwriting each other
        path = os.path.join(directory, f"{time.time_ns()}-{event}.json")
        try:
            fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o644)
            break
        except FileExistsError:
            continue
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        json.dump({"event": event, "stdin": raw, "stdout": out}, f)
    return out


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if len(argv) != 2:
        print("usage: capture_hook.py <dir> <nonce-file>", file=sys.stderr)
        return 64
    raw = sys.stdin.buffer.read().decode("utf-8", errors="replace")
    out = capture(argv[0], argv[1], raw)
    if out:
        print(out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
