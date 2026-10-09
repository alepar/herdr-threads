#!/usr/bin/env bash
# release_contract.sh — replay indexed representable legacy payload captures with
# the built release's own discovery (old releases explicitly use contract-id).
# Exact build/native-shape/bridge domains without a release replay evaluator are
# per-domain unsupported. Original identity, attempt and stage accompany results;
# schema replay never upgrades no_model to live. Infrastructure errors exit 2 and
# write an error document so the writer preserves all baseline collections.
# Usage: release_contract.sh <release-tag> <canary-out-dir>
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
# The built release is the sole authority for its discovery. Older releases
# explicitly fall back to contract-id; unsupported rich domains stay separate.
MANIFEST_PY=$SCRIPT_DIR/manifest.py
QUERY=$OUT/release-discovery.json
python3 - "$MANIFEST_PY" "$BUILT" > "$QUERY" <<'PYQUERY' || die "release discovery failed"
import importlib.util, json, sys
script, binary = sys.argv[1:]
spec = importlib.util.spec_from_file_location("release_manifest", script)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
registry = m.discovery(binary)
if registry is None:
    try:
        legacy = m.contract_ids(binary)
        ids = {h: legacy[h] for h in m.HARNESSES}
    except ValueError:
        print(json.dumps({"supported": False}))
        sys.exit(0)
else:
    ids = {a["id"]: a["legacy_contract_id"] for a in registry["adapters"] if a["legacy_contract_id"] is not None}
print(json.dumps({"supported": True, "discovery": registry, "contract_id": ids}))
PYQUERY
python3 - "$QUERY" <<'PYSUPPORTED' || unsupported "discovery and legacy contract-id unavailable"
import json, sys
sys.exit(0 if json.load(open(sys.argv[1]))["supported"] else 1)
PYSUPPORTED
rm -rf "$OUT/release-work"
PLAN=$OUT/release-plan.json
python3 - "$MANIFEST_PY" "$OUT" "$QUERY" > "$PLAN" <<'PYPLAN' || die "invalid indexed source artifacts"
import importlib.util, json, sys
script, root, query = sys.argv[1:]
spec = importlib.util.spec_from_file_location("release_manifest", script)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
query = json.load(open(query))
print(json.dumps(m.prepare_replay(root, query["discovery"], query["contract_id"]), indent=2))
PYPLAN
LIST=$OUT/release-probes.tsv
python3 - "$PLAN" > "$LIST" <<'PYLIST'
import json, sys
for p in json.load(open(sys.argv[1]))["probes"]:
    print(p["harness"] + "\t" + p["version"] + "\t" + p["work_path"])
PYLIST

cargo_home=${CARGO_HOME:-$REAL_HOME/.cargo}
rustup_home=${RUSTUP_HOME:-$REAL_HOME/.rustup}
mkdir -p "$OUT/release-work"
while IFS=$'\t' read -r h v name; do
  [ -n "$name" ] || continue
  W=$OUT/$name
  mkdir -p "$W/home" "$W/tmp"
  e=(env -i "HOME=$W/home" "CLAUDE_CONFIG_DIR=$W/home/.claude" "CODEX_HOME=$W/home/.codex"
    "XDG_CONFIG_HOME=$W/home/.config" "XDG_STATE_HOME=$W/home/.local/state"
    "XDG_DATA_HOME=$W/home/.local/share" "XDG_CACHE_HOME=$W/home/.cache" "TMPDIR=$W/tmp"
    "PATH=$cargo_home/bin:$(dirname "$CARGO_BIN"):/usr/bin:/bin" "LANG=C.UTF-8" "TERM=dumb"
    "CARGO_HOME=$cargo_home" "RUSTUP_HOME=$rustup_home" "CARGO_TARGET_DIR=$TARGET"
    "HT_CANARY_CAPTURE_DIR=$W" "PYTHONDONTWRITEBYTECODE=1")
  if [ -n "${RUSTUP_TOOLCHAIN:-}" ]; then e+=("RUSTUP_TOOLCHAIN=$RUSTUP_TOOLCHAIN"); fi
  rc=0
  (cd "$SRC" && python3 "$RUN_PY" --timeout 1800 -- "${e[@]}" nice cargo test --locked --all-features --lib \
    canary_payloads -- --nocapture >"$W/payload-parse.out" 2>"$W/payload-parse.err" </dev/null) || rc=$?
  # a release test that failed or never ran leaves no canary-rust.json: that probe is simply not collected
  [ "$rc" -eq 0 ] || echo "release_contract.sh: canary_payloads at $TAG exited $rc for $h $v (see $W)" >&2
done < "$LIST"

python3 - "$MANIFEST_PY" "$TAG" "$OUT" "$PLAN" > "$RESULT" <<'PYRESULT'
import importlib.util, json, sys
script, tag, root, plan = sys.argv[1:]
spec = importlib.util.spec_from_file_location("release_manifest", script)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
print(json.dumps(m.collect_replay(root, json.load(open(plan)), tag), indent=2))
PYRESULT
