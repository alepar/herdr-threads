#!/usr/bin/env bash
# release_contract.sh — run the latest release's payload contract over the canary's kept probe captures.
#
#   release_contract.sh <release-tag> <canary-out-dir>
#
# The canary keeps one row per live contract (docs/compatibility/harnesses.md, "Manifest publishing"): the
# current tree's contract and the latest release's. This script adds a git worktree of <release-tag> under
# <canary-out-dir>/release-src, builds it (`cargo build --locked`) and asks the built binary for its
# `contract-id --json`. A release that predates contract ids cannot answer: the script then writes
# {"tag": "<tag>", "supported": false} to <canary-out-dir>/release-contract.json and exits 0. An infrastructure
# failure (checkout, no cargo, a build failure, no binary built) is not "unsupported": once the result path is
# known the script writes {"tag": "<tag>", "error": "<reason>"} instead and exits 2, and the manifest writer keeps
# the baseline's release-contract rows.
#
# Otherwise, for every probe work directory `--keep` left under <canary-out-dir>/work/<harness>-<version>-<n>/
# (the highest <n> per harness and version), it copies the probe's capture into <canary-out-dir>/release-work/
# and runs the release tree's gated `canary_payloads` test over the copy (same `env -i` isolation as
# harness-canary.sh's t0.payload-parse; CARGO_TARGET_DIR=<canary-out-dir>/release-target), so the main run's
# canary-rust.json files are never overwritten. The payload classifications of each copy's canary-rust.json are
# collected into
#   {"tag", "supported": true, "contract_id": {"claude", "codex"},
#    "probes": [{"harness", "version", "payloads": [{"event", "kind", "field"}]}]}
# A payload the release tree's test did not classify (its canary_payloads predates classification) is recorded
# with kind "unclassified", which never verifies and never breaks a row. The worktree is removed on exit.
# Three result forms: supported (above), {"tag", "supported": false}, {"tag", "error"}.
# Exit: 0 on a written supported or unsupported release-contract.json, 2 on bad arguments or an infrastructure
# error (an error document is written when the failure came after the result path was set).
set -euo pipefail

die() { echo "release_contract.sh: $*" >&2; exit 2; }

[ $# -eq 2 ] || die "usage: release_contract.sh <release-tag> <canary-out-dir>"
TAG=$1
OUT=$2
[ -d "$OUT" ] || die "no such directory: $OUT"
OUT=$(cd "$OUT" && pwd -P)
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
ROOT=$(cd "$SCRIPT_DIR/../.." && pwd -P)
RUN_PY=$SCRIPT_DIR/run.py
SRC=$OUT/release-src
RESULT=$OUT/release-contract.json
REAL_HOME=${HOME:-}

# from here an infrastructure failure also leaves an error document, which the manifest writer treats as "says
# nothing about the contract" (the baseline's release rows stay)
die() {
  echo "release_contract.sh: $*" >&2
  python3 -c 'import json,sys; print(json.dumps({"tag": sys.argv[1], "error": sys.argv[2]}, indent=2))' "$TAG" "$*" > "$RESULT" 2>/dev/null || true
  exit 2
}

cleanup() {
  if [ -d "$SRC" ]; then git -C "$ROOT" worktree remove --force "$SRC" >/dev/null 2>&1 || rm -rf "$SRC"; fi
  git -C "$ROOT" worktree prune >/dev/null 2>&1 || true
}
trap cleanup EXIT

unsupported() {
  python3 -c 'import json, sys; print(json.dumps({"tag": sys.argv[1], "supported": False}, indent=2))' "$TAG" > "$RESULT"
  echo "release_contract.sh: $TAG cannot report a contract id ($1); wrote supported:false" >&2
  exit 0
}

rm -rf "$SRC"
git -C "$ROOT" worktree add --detach "$SRC" "$TAG" >/dev/null 2>&1 || die "cannot check out $TAG"
CARGO_BIN=$(command -v cargo || true)
[ -n "$CARGO_BIN" ] || die "cargo not found"
TARGET=$OUT/release-target
mkdir -p "$OUT/release-logs"
(cd "$SRC" && CARGO_TARGET_DIR=$TARGET cargo build --locked >"$OUT/release-logs/build.out" 2>"$OUT/release-logs/build.err") \
  || die "build failed"
BUILT=$TARGET/debug/herdr-threads
[ -x "$BUILT" ] || die "no binary was built"
if ! IDS=$("$BUILT" contract-id --json 2>"$OUT/release-logs/contract-id.err"); then unsupported "contract-id failed"; fi
printf '%s' "$IDS" | python3 -c 'import json, sys; d = json.load(sys.stdin); assert isinstance(d.get("claude"), str) and isinstance(d.get("codex"), str)' \
  2>/dev/null || unsupported "contract-id printed no ids"

# one kept work directory per (harness, version): the highest attempt number
LIST=$OUT/release-probes.tsv
python3 - "$OUT/work" > "$LIST" <<'PY'
import os, re, sys
best = {}
if os.path.isdir(sys.argv[1]):
    for name in os.listdir(sys.argv[1]):
        m = re.fullmatch(r"(claude|codex)-(\d+\.\d+\.\d+)-(\d+)", name)
        if m and os.path.isfile(os.path.join(sys.argv[1], name, "canary-probe.json")):
            key = (m.group(1), m.group(2))
            if key not in best or int(m.group(3)) > best[key][0]:
                best[key] = (int(m.group(3)), name)
for (h, v), (_, name) in sorted(best.items()):
    print(f"{h}\t{v}\t{name}")
PY

cargo_home=${CARGO_HOME:-$REAL_HOME/.cargo}
rustup_home=${RUSTUP_HOME:-$REAL_HOME/.rustup}
rm -rf "$OUT/release-work"
mkdir -p "$OUT/release-work"
while IFS=$'\t' read -r h v name; do
  [ -n "$name" ] || continue
  W=$OUT/release-work/$h-$v
  mkdir -p "$W/home" "$W/tmp"
  cp "$OUT/work/$name/canary-probe.json" "$W/canary-probe.json"
  for sub in capture help; do
    if [ -d "$OUT/work/$name/$sub" ]; then cp -R "$OUT/work/$name/$sub" "$W/$sub"; fi
  done
  e=(env -i "HOME=$W/home" "CLAUDE_CONFIG_DIR=$W/home/.claude" "CODEX_HOME=$W/home/.codex"
    "XDG_CONFIG_HOME=$W/home/.config" "XDG_STATE_HOME=$W/home/.local/state"
    "XDG_DATA_HOME=$W/home/.local/share" "XDG_CACHE_HOME=$W/home/.cache" "TMPDIR=$W/tmp"
    "PATH=$cargo_home/bin:$(dirname "$CARGO_BIN"):/usr/bin:/bin" "LANG=C.UTF-8" "TERM=dumb"
    "CARGO_HOME=$cargo_home" "RUSTUP_HOME=$rustup_home" "CARGO_TARGET_DIR=$TARGET"
    "HT_CANARY_CAPTURE_DIR=$W")
  if [ -n "${RUSTUP_TOOLCHAIN:-}" ]; then e+=("RUSTUP_TOOLCHAIN=$RUSTUP_TOOLCHAIN"); fi
  rc=0
  (cd "$SRC" && python3 "$RUN_PY" --timeout 1800 -- "${e[@]}" nice cargo test --locked --all-features --lib \
    canary_payloads -- --nocapture >"$W/payload-parse.out" 2>"$W/payload-parse.err" </dev/null) || rc=$?
  # a release test that failed or never ran leaves no canary-rust.json: that probe is simply not collected
  [ "$rc" -eq 0 ] || echo "release_contract.sh: canary_payloads at $TAG exited $rc for $h $v (see $W)" >&2
done < "$LIST"

python3 - "$TAG" "$OUT/release-work" "$LIST" "$IDS" > "$RESULT" <<'PY'
import json, os, sys
tag, work, listing, ids = sys.argv[1:5]
ids = json.loads(ids)
probes = []
for line in open(listing, encoding="utf-8"):
    line = line.rstrip("\n")
    if not line:
        continue
    h, v, _ = line.split("\t")
    path = os.path.join(work, f"{h}-{v}", "canary-rust.json")
    try:
        rust = json.load(open(path, encoding="utf-8"))
    except (OSError, ValueError):
        continue
    payloads = []
    for p in rust.get("payloads", []):
        c = p.get("contract") if isinstance(p, dict) else None
        if isinstance(c, dict) and c.get("kind") in ("ok", "violation", "malformed"):
            payloads.append({"event": c.get("event"), "kind": c["kind"], "field": c.get("field")})
        else:
            payloads.append({"event": p.get("event") or None, "kind": "unclassified", "field": None})
    probes.append({"harness": h, "version": v, "payloads": payloads})
print(json.dumps({"tag": tag, "supported": True, "contract_id": {"claude": ids["claude"], "codex": ids["codex"]},
                  "probes": probes}, indent=2))
PY
