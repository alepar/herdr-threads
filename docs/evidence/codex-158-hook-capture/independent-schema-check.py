#!/usr/bin/env python3
"""Independent re-extraction of the hook schemas embedded in installed Codex
binaries. Finds every draft-07 object titled `<event>.command.(input|output)`,
parses it as JSON, and hashes its canonical (sorted-key) form. Read-only."""
import hashlib, json, os, re, sys

RELEASES = os.path.expanduser("~/.codex/packages/standalone/releases/")
VERSIONS = sys.argv[1:] or ["0.155.1", "0.157.1", "0.158.0"]


def extract(path):
    data = open(path, "rb").read()
    out = {}
    for m in re.finditer(rb'"title": "([a-z-]+\.command\.(?:input|output))"', data):
        title = m.group(1).decode()
        i, depth = m.start(), 0
        while i > 0:
            i -= 1
            c = data[i:i + 1]
            if c == b"}":
                depth += 1
            elif c == b"{":
                if depth == 0:
                    if data[i:i + 40].lstrip(b"{\n ").startswith(b'"$schema"'):
                        break
                else:
                    depth -= 1
        j, depth = i, 0
        while True:
            c = data[j:j + 1]
            if c == b"{":
                depth += 1
            elif c == b"}":
                depth -= 1
                if depth == 0:
                    break
            j += 1
        try:
            obj = json.loads(data[i:j + 1])
        except ValueError:
            continue
        digest = hashlib.sha256(json.dumps(obj, sort_keys=True).encode()).hexdigest()
        out.setdefault(title, set()).add(digest)
    return out


found = {}
for v in VERSIONS:
    binary = f"{RELEASES}{v}-aarch64-apple-darwin/codex"
    found[v] = extract(binary)
    print(v, "sha256", hashlib.sha256(open(binary, "rb").read()).hexdigest(), "schemas", len(found[v]))
base = found[VERSIONS[-1]]
for v in VERSIONS[:-1]:
    print(f"{v} == {VERSIONS[-1]}:", found[v] == base)
for title in sorted(base):
    print(title, ",".join(sorted(d[:16] for d in base[title])))
