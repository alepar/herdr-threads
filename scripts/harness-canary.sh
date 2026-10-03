#!/usr/bin/env bash
# harness-canary.sh — native harness version canary (nested spec §D1–§D5, §D8).
#
#   harness-canary.sh [--harness claude|codex|both] [--versions latest|since-verified|V1,V2,...]
#                     [--bisect] [--model-tier auto|off|required] [--out DIR] [--baseline V]
#                     [--herdr-threads PATH] [--versions-json PATH] [--baseline-json PATH] [--keep]
#   harness-canary.sh --probe HARNESS VERSION [same options]   one probe; prints a probeResult JSON
#   harness-canary.sh --self-test                              offline self-test (no network, npm, cargo)
#   harness-canary.sh --write-probe-files P HARNESS VERSION BINARY
#   harness-canary.sh --check-admission DOCTOR_JSON CANARY_PROBE_JSON   t0.admission on saved files; prints status<TAB>detail<TAB>admission
#
# Exit: 0 every harness all_pass / no_candidates / known_broken_persists; 1 any break or inconclusive;
# 2 any infra error, bad arguments or an isolation refusal. In --probe mode: 0 pass, 1 fail, 2 infra.
#
# Test-only: HT_CANARY_PLANT_BREAK=<harness>@<version> adds a failing t0.planted check to every probe of
# that harness at a version >= <version>. --versions-json PATH replaces docs/compatibility/harness-versions.json
# for verified_max / known_broken / expected_admission, and the code under test then honors
# HT_TEST_RECIPES_JSON=PATH (build with --features test-support).
# HT_CANARY_PLANT_TIER1_FAIL=<harness>@<version>[:count] makes the tier-1 call site of that exact probe emit a
# failing t1.planted check (failed_tier 1) and make no model call; with :count only the first COUNT probes of
# that harness@version fail (counted in the --out work directory), without it every probe does.
#
# Tier 1 (model, §D6): --model-tier auto runs it for a harness only when its key (ANTHROPIC_API_KEY for claude,
# OPENAI_API_KEY for codex) is non-empty in the canary's own environment; off never; required makes a missing
# key an infra error (exit 2). scripts/canary/tier1.sh holds the runs.
#
# Tier 0 checks are table-driven: a check is a function chk_<id with . and - as _> that sets CK_STATUS
# (pass|fail|warn|skip) and CK_DETAIL; TIER0_CHECKS lists them in run order. The t0.isolation tripwire always
# runs last, after everything else has had its chance to touch the real config.
#
# Test-only: HT_CANARY_TIER0_CHECKS="id id ..." replaces the tier-0 check list, so a test of another part of the
# script (tier 1) can run against fake binaries that do not implement setup/doctor/hooks.
# Test-only: HT_CANARY_SOURCE_ONLY=1 makes sourcing this file stop after the function definitions (before any
# probe or orchestration runs), so scripts/canary/test_tier0_checks.py can drive single checks.
# shellcheck disable=SC2329  # the chk_* functions are invoked indirectly by run_check
set -euo pipefail

SCRIPT_PATH=$0
case $SCRIPT_PATH in /*) ;; *) SCRIPT_PATH=$PWD/$SCRIPT_PATH ;; esac
SCRIPT_DIR=$(cd "$(dirname "$SCRIPT_PATH")" && pwd -P)
SCRIPT_PATH=$SCRIPT_DIR/$(basename "$SCRIPT_PATH")
ROOT=$(cd "$SCRIPT_DIR/.." && pwd -P)
CANARY=$ROOT/scripts/canary
REAL_HOME=${HOME:-}
# Tier-1 keys come only from the canary's own environment (§D6), never from aisw profiles or harness homes.
KEY_ANTHROPIC=${ANTHROPIC_API_KEY:-}
KEY_OPENAI=${OPENAI_API_KEY:-}

HARNESS=both
VERSIONS=since-verified
BISECT=0
MODEL_TIER=auto
OUT=
OUT_IS_TEMP=0
BASELINE=
HT_BIN=
VERSIONS_JSON=
BASELINE_JSON=
KEEP=0
MODE=run
PROBE_HARNESS=
PROBE_VERSION=
WPF_ARGS=()
CHECK_ARGS=()

die() { echo "harness-canary.sh: $*" >&2; exit 2; }
usage() { sed -n '2,13p' "$SCRIPT_PATH" | sed 's/^# \{0,1\}//' >&2; }

json_str() {
  local s=$1
  s=${s//\\/\\\\}
  s=${s//\"/\\\"}
  s=${s//$'\n'/\\n}
  s=${s//$'\r'/\\r}
  s=${s//$'\t'/\\t}
  printf '"%s"' "$s"
}

# ---------------------------------------------------------------- arguments

while [ $# -gt 0 ]; do
  case $1 in
    --harness) [ $# -ge 2 ] || die "--harness needs a value"; HARNESS=$2; shift 2 ;;
    --versions) [ $# -ge 2 ] || die "--versions needs a value"; VERSIONS=$2; shift 2 ;;
    --bisect) BISECT=1; shift ;;
    --model-tier) [ $# -ge 2 ] || die "--model-tier needs a value"; MODEL_TIER=$2; shift 2 ;;
    --out) [ $# -ge 2 ] || die "--out needs a value"; OUT=$2; shift 2 ;;
    --baseline) [ $# -ge 2 ] || die "--baseline needs a value"; BASELINE=$2; shift 2 ;;
    --herdr-threads) [ $# -ge 2 ] || die "--herdr-threads needs a value"; HT_BIN=$2; shift 2 ;;
    --versions-json) [ $# -ge 2 ] || die "--versions-json needs a value"; VERSIONS_JSON=$2; shift 2 ;;
    --baseline-json) [ $# -ge 2 ] || die "--baseline-json needs a value"; BASELINE_JSON=$2; shift 2 ;;
    --keep) KEEP=1; shift ;;
    --self-test) MODE=self-test; shift ;;
    --probe)
      [ $# -ge 3 ] || { usage; die "--probe needs HARNESS VERSION"; }
      MODE=probe; PROBE_HARNESS=$2; PROBE_VERSION=$3; shift 3 ;;
    --write-probe-files)
      [ $# -ge 5 ] || { echo "usage: $0 --write-probe-files P HARNESS VERSION BINARY" >&2; exit 64; }
      MODE=write-probe-files; WPF_ARGS=("$2" "$3" "$4" "$5"); shift 5 ;;
    --check-admission)
      [ $# -ge 3 ] || { usage; die "--check-admission needs DOCTOR_JSON CANARY_PROBE_JSON"; }
      MODE=check-admission; CHECK_ARGS=("$2" "$3"); shift 3 ;;
    -h|--help) usage; exit 0 ;;
    *) usage; die "unknown argument: $1" ;;
  esac
done

case $HARNESS in claude|codex|both) ;; *) die "--harness must be claude, codex or both" ;; esac
case $MODEL_TIER in auto|off|required) ;; *) die "--model-tier must be auto, off or required" ;; esac
if [ -n "$BASELINE" ]; then
  [[ $BASELINE =~ ^(0|[1-9][0-9]{0,5})\.(0|[1-9][0-9]{0,5})\.(0|[1-9][0-9]{0,5})$ ]] || die "--baseline must be X.Y.Z"
fi

abs_path() { python3 -c 'import os,sys; print(os.path.realpath(os.path.expanduser(sys.argv[1])))' "$1"; }

if [ -n "$VERSIONS_JSON" ]; then
  [ -f "$VERSIONS_JSON" ] || die "--versions-json: no such file: $VERSIONS_JSON"
  VERSIONS_JSON=$(abs_path "$VERSIONS_JSON")
fi
VJSON=${VERSIONS_JSON:-$ROOT/docs/compatibility/harness-versions.json}
# --baseline-json PATH (the published harness-manifest branch file) feeds candidate selection only
# (since-verified baseline and known-broken exclusion); it never replaces the recipes under test.
if [ -n "$BASELINE_JSON" ]; then
  [ -f "$BASELINE_JSON" ] || die "--baseline-json: no such file: $BASELINE_JSON"
  BASELINE_JSON=$(abs_path "$BASELINE_JSON")
fi
BJSON=${BASELINE_JSON:-$VJSON}
[ -z "$HT_BIN" ] || { [ -x "$HT_BIN" ] || die "--herdr-threads: not an executable: $HT_BIN"; HT_BIN=$(abs_path "$HT_BIN"); }

if [ "$MODE" = self-test ]; then
  exec python3 "$ROOT/scripts/harness-canary-selftest/run.py"
fi

# ---------------------------------------------------------------- versions.py helpers

# canary_py SUBCOMMAND ARGS... — thin wrapper over scripts/canary/{versions,report}.py.
canary_py() {
  CANARY_DIR=$CANARY python3 - "$@" <<'PY'
import importlib.util, json, os, sys

def load(name):
    mod = sys.modules.get(f"canary_{name}")
    if mod is None:
        spec = importlib.util.spec_from_file_location(f"canary_{name}", os.path.join(os.environ["CANARY_DIR"], f"{name}.py"))
        mod = importlib.util.module_from_spec(spec)
        sys.modules[f"canary_{name}"] = mod
        spec.loader.exec_module(mod)
    return mod

versions = load("versions")
cmd, args = sys.argv[1], sys.argv[2:]
try:
    if cmd == "expected":  # H V JSON -> expected_admission (§D3 t0.admission), never influenced by --baseline
        harness, version, path = args
        doc = versions.load_versions_json(path)
        if versions.in_any_range(version, versions.known_broken(doc, harness)):
            print("refused")
        elif any(r.get("version") == version for r in doc["rows"] if r.get("harness") == harness):
            print("listed")
        else:
            vmax = versions.verified_max(doc, harness)
            if vmax is not None and versions.version_key(version) > versions.version_key(vmax):
                print("optimistic" if harness == "claude" else "schema-matched-or-optimistic")
            else:
                print("unasserted")
    elif cmd == "verified-max":  # H JSON [MAIN_ID]
        print(versions.verified_max(versions.load_versions_json(args[1]), args[0], (args[2:3] or [""])[0] or None) or "")
    elif cmd == "known-broken":  # H JSON [MAIN_ID] -> JSON list of {min,max}
        rngs = versions.known_broken(versions.load_versions_json(args[1]), args[0], (args[2:3] or [""])[0] or None)
        print(json.dumps([{"min": lo, "max": hi} for lo, hi in rngs]))
    elif cmd == "reprobe":  # H JSON NPMLIST_FILE MAIN_ID -> comma-separated versions known_broken under another contract
        harness, path, listfile, main_id = args
        npm_list = json.load(open(listfile))
        if isinstance(npm_list, str):
            npm_list = [npm_list]
        print(",".join(versions.reprobe(npm_list, versions.load_versions_json(path), harness, main_id or None)))
    elif cmd == "candidates":  # H MODE JSON EXPLICIT NPMLIST_FILE [MAIN_ID]
        harness, mode, path, explicit, listfile = args[:5]
        main_id = (args[5:6] or [""])[0] or None
        npm_list = json.load(open(listfile))
        if isinstance(npm_list, str):
            npm_list = [npm_list]
        doc = versions.load_versions_json(path)
        ex = [v for v in explicit.split(",") if v] if explicit else None
        print(",".join(versions.candidates(npm_list, mode, doc, harness, ex, main_id)))
    elif cmd == "probe-json":  # TSV_FILE FORCED_RESULT [CANARY_RUST_JSON...] -> probeResult on stdout, exit code via stderr
        tsv, forced = args[:2]
        # The per-payload contract verdicts of the gated canary_payloads runs (tier 0, then tier 1); the field is
        # omitted when neither run wrote a report (the probe never got that far).
        contract = None
        for rust in args[2:]:
            try:
                rdoc = json.load(open(rust, encoding="utf-8"))
            except (OSError, ValueError):
                continue
            if not isinstance(rdoc, dict):
                continue
            contract = contract or {"contract_id": rdoc.get("contract_id"), "payloads": [], "release": None}
            for pl in rdoc.get("payloads", []):
                c = pl.get("contract") if isinstance(pl, dict) else None
                if isinstance(c, dict) and c.get("kind") in ("ok", "violation", "malformed"):
                    contract["payloads"].append({"event": c.get("event"), "kind": c["kind"], "field": c.get("field")})
        checks = []
        for line in open(tsv, encoding="utf-8"):
            line = line.rstrip("\n")
            if not line:
                continue
            cid, status, detail = (line.split("\t", 2) + ["", ""])[:3]
            c = {"id": cid, "status": status}
            if detail:
                c["detail"] = detail
            checks.append(c)
        failed = any(c["status"] == "fail" for c in checks)
        if forced == "infra":
            result, tier = "infra", None
        elif failed:
            # a failing tier-1 check marks the probe failed_tier 1 (bisect retries it twice, §D6/§D7)
            result = "fail"
            tier = 1 if any(c["status"] == "fail" and c["id"].startswith("t1.") for c in checks) else 0
        else:
            result, tier = "pass", None
        doc = {"result": result, "checks": checks, "failed_tier": tier}
        if contract is not None:
            doc["contract"] = contract
        print(json.dumps(doc))
        print({"pass": 0, "fail": 1, "infra": 2}[result], file=sys.stderr)
    elif cmd == "report":  # OUT HARNESSES_CSV INPUTS_JSON COMMIT HT_VERSION OS ARCH JSON BASELINE [MAIN_IDS_JSON]
        out, hs, inputs, commit, htv, osn, arch, path, baseline = args[:9]
        main_ids = json.loads(args[9]) if len(args) > 9 and args[9] else {}
        report = load("report")
        doc = versions.load_versions_json(path)
        blocks = []
        for h in hs.split(","):
            res = json.load(open(os.path.join(out, f"bisect-{h}.json")))
            vmax = versions.verified_max(doc, h, main_ids.get(h))
            blocks.append(report.harness_block(h, res, verified_max=vmax or baseline or None))
        rep = report.assemble(blocks, json.loads(inputs), {"os": osn, "arch": arch}, commit, htv)
        report.write(rep, out)
        print(rep["exit_code"])
    else:
        raise SystemExit(f"unknown subcommand {cmd}")
except (ValueError, OSError, KeyError) as e:
    print(f"harness-canary.sh: {e}", file=sys.stderr)
    sys.exit(2)
PY
}

write_canary_probe() {
  local p=$1 harness=$2 version=$3 binary=$4 expected=$5
  mkdir -p "$p"
  printf '{"harness":%s,"version":%s,"binary":%s,"expected_admission":%s}\n' \
    "$(json_str "$harness")" "$(json_str "$version")" "$(json_str "$binary")" "$(json_str "$expected")" \
    > "$p/canary-probe.json"
}

write_help_texts() {
  local p=$1 codex_bin=$2
  mkdir -p "$p/help"
  "$codex_bin" --help > "$p/help/codex.txt" 2>&1 || true
  "$codex_bin" exec --help > "$p/help/codex-exec.txt" 2>&1 || true
}

if [ "$MODE" = write-probe-files ]; then
  wp=${WPF_ARGS[0]}; wh=${WPF_ARGS[1]}; wv=${WPF_ARGS[2]}; wb=${WPF_ARGS[3]}
  expected=$(canary_py expected "$wh" "$wv" "$VJSON") || exit 2
  write_canary_probe "$wp" "$wh" "$wv" "$wb" "$expected"
  [ "$wh" != codex ] || write_help_texts "$wp" "$wb"
  exit 0
fi

# --check-admission DOCTOR PROBE: the t0.admission verdict on a saved `doctor --json` and the probe's
# canary-probe.json (harness, expected_admission). Exit 0 pass/skip, 1 fail, 2 infra (not_found).
if [ "$MODE" = check-admission ]; then
  [ -f "${CHECK_ARGS[0]}" ] || die "--check-admission: no such file: ${CHECK_ARGS[0]}"
  [ -f "${CHECK_ARGS[1]}" ] || die "--check-admission: no such file: ${CHECK_ARGS[1]}"
  probe_fields=$(python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(d["harness"], d["expected_admission"])' "${CHECK_ARGS[1]}") \
    || die "--check-admission: ${CHECK_ARGS[1]} is not a canary-probe.json"
  read -r ca_harness ca_expected <<<"$probe_fields"
  ca_line=$(python3 "$CANARY/admission.py" evaluate --harness "$ca_harness" --expected "$ca_expected" "${CHECK_ARGS[0]}")
  printf '%s\n' "$ca_line"
  case ${ca_line%%$'\t'*} in pass|skip) exit 0 ;; infra) exit 2 ;; *) exit 1 ;; esac
fi

# ---------------------------------------------------------------- output directory and refusals

# ---------------------------------------------------------------- tier-1 key gate (§D6)

tier1_keyvar() { case $1 in claude) echo ANTHROPIC_API_KEY ;; codex) echo OPENAI_API_KEY ;; esac; }
tier1_key() { case $1 in claude) printf '%s' "$KEY_ANTHROPIC" ;; codex) printf '%s' "$KEY_OPENAI" ;; esac; }
# tier1_enabled HARNESS: tier 1 runs for it (not off, key present).
tier1_enabled() { [ "$MODEL_TIER" != off ] && [ -n "$(tier1_key "$1")" ]; }

if [ "$MODEL_TIER" = required ]; then
  case $MODE in probe) gate_harnesses=$PROBE_HARNESS ;; *) gate_harnesses=${HARNESS/both/claude codex} ;; esac
  for gh in $gate_harnesses; do
    [ -n "$(tier1_keyvar "$gh")" ] || continue
    [ -n "$(tier1_key "$gh")" ] || die "--model-tier required: $(tier1_keyvar "$gh") is empty or unset (needed for $gh)"
  done
fi

if [ -z "$OUT" ]; then
  OUT=$(mktemp -d "${TMPDIR:-/tmp}/hc-XXXXXX")
  OUT_IS_TEMP=1
fi
OUT=$(abs_path "$OUT")

refuse_out() {
  local out=$1 forbidden
  for forbidden in "$REAL_HOME/.claude" "$REAL_HOME/.codex" "$REAL_HOME/.aisw"; do
    [ -n "$REAL_HOME" ] || break
    local f; f=$(abs_path "$forbidden")
    case $out/ in "$f"/*) die "--out $out is inside $forbidden; the canary never writes there" ;; esac
  done
  case $out/ in
    "$ROOT"/*)
      local rel=${out#"$ROOT"}; rel=${rel#/}; rel=${rel:-.}
      if [ -n "$(git -C "$ROOT" ls-files -- "$rel" 2>/dev/null)" ]; then
        die "--out $out is inside the tracked tree"
      fi
      git -C "$ROOT" check-ignore -q -- "$rel" 2>/dev/null \
        || die "--out $out is inside the repository checkout and not git-ignored"
      ;;
  esac
}
refuse_out "$OUT"
mkdir -p "$OUT/work"

NODE_BIN=$(command -v node || true)
NPM_BIN=$(command -v npm || true)
[ -n "$NODE_BIN" ] && [ -n "$NPM_BIN" ] || die "node and npm are required"
NODE_DIR=$(dirname "$NODE_BIN")
CARGO_BIN=$(command -v cargo || true)
RUN_PY=$CANARY/run.py

# ---------------------------------------------------------------- isolated environment (§D2)

ENVV=()
# build_env DIR: ENVV = `env -i ...` with HOME and every config/state/tmp directory inside DIR.
build_env() {
  local d=$1
  mkdir -p "$d/home/.claude" "$d/home/.codex" "$d/home/.config" "$d/home/.local/state" \
    "$d/home/.local/share" "$d/home/.cache" "$d/tmp" "$d/bin"
  ENVV=(env -i "HOME=$d/home" "CLAUDE_CONFIG_DIR=$d/home/.claude" "CODEX_HOME=$d/home/.codex"
    "XDG_CONFIG_HOME=$d/home/.config" "XDG_STATE_HOME=$d/home/.local/state"
    "XDG_DATA_HOME=$d/home/.local/share" "XDG_CACHE_HOME=$d/home/.cache" "TMPDIR=$d/tmp"
    "PATH=$d/bin:$NODE_DIR:/usr/bin:/bin" "LANG=C.UTF-8" "TERM=dumb")
  if [ -n "$VERSIONS_JSON" ]; then ENVV+=("HT_TEST_RECIPES_JSON=$VERSIONS_JSON"); fi
}

XRC=0
# xrun LABEL TIMEOUT CMD...: run CMD under ENVV in its own process group via run.py; stdout/stderr go to
# $LOGS/LABEL.{out,err}; the exit code lands in XRC (never aborts the script).
xrun() {
  local label=$1 timeout=$2; shift 2
  XRC=0
  python3 "$RUN_PY" --timeout "$timeout" -- "${ENVV[@]}" "$@" >"$LOGS/$label.out" 2>"$LOGS/$label.err" </dev/null || XRC=$?
}

tail_of() { tail -c 400 "$1" 2>/dev/null | tr '\n\t' '  ' || true; }

# ---------------------------------------------------------------- herdr-threads under test

ensure_ht() {
  [ -n "$HT_BIN" ] && return 0
  if [ -z "$CARGO_BIN" ]; then return 1; fi
  local feat=()
  if [ -n "$VERSIONS_JSON" ]; then feat=(--features test-support); fi
  echo "harness-canary.sh: building herdr-threads (cargo build --locked) ..." >&2
  (cd "$ROOT" && nice cargo build --locked ${feat[@]+"${feat[@]}"} >&2) || return 1
  HT_BIN=${CARGO_TARGET_DIR:-$ROOT/target}/debug/herdr-threads
  [ -x "$HT_BIN" ]
}

# ---------------------------------------------------------------- tier 0 (§D3)

HT_S=()
HOOK_FIRES_RAN=0         # set by t0.hook-fires; t0.payload-parse then requires a hook capture
SCHEMA_RESULT=''         # match|drift|unextractable, set by t0.schema (codex)
ADMISSION_OBSERVED=''    # doctor's admission string, set by t0.admission
EXPECTED_ADMISSION=unasserted

# Order matters: setup before config-load/hook-fires (they run the setup-written configuration); hook-fires
# before payload-parse (it parses the captures); t0.schema before t0.admission (a Codex version above
# verified_max is finalized against it, coverage r2); doctor checks and unsetup after the witness is swapped
# back out, so none of them is captured into capture/tier0.
TIER0_CHECKS=(t0.version t0.setup t0.config-load t0.hook-fires t0.payload-parse t0.schema t0.admission
  t0.launch-flags t0.npm-shim-admission t0.launch-tables t0.unsetup)
if [ -n "${HT_CANARY_TIER0_CHECKS:-}" ]; then read -r -a TIER0_CHECKS <<<"$HT_CANARY_TIER0_CHECKS"; fi
if [ -n "${HT_CANARY_PLANT_BREAK:-}" ]; then TIER0_CHECKS+=(t0.planted); fi

version_ge() { # A B: A >= B for X.Y.Z
  python3 -c 'import sys; k=lambda v: tuple(int(p) for p in v.split(".")); sys.exit(0 if k(sys.argv[1])>=k(sys.argv[2]) else 1)' "$1" "$2"
}

chk_t0_version() {
  local want
  if [ "$H" = claude ]; then want="$V (Claude Code)"; else want="codex-cli $V"; fi
  xrun version 15 "$P/bin/$H" --version
  local got; got=$(head -n 1 "$LOGS/version.out" 2>/dev/null | tr -d '\r' || true)
  if [ "$XRC" -ne 0 ]; then CK_STATUS=fail; CK_DETAIL="--version exited $XRC: $(tail_of "$LOGS/version.err")"
  elif [ "$got" != "$want" ]; then CK_STATUS=fail; CK_DETAIL="--version printed '$got', expected '$want'"
  else CK_STATUS=pass; CK_DETAIL=$got; fi
}

chk_t0_config_load() {
  if [ "$H" = claude ]; then
    CK_STATUS=skip; CK_DETAIL="covered by t0.hook-fires (settings.json is only read by a session)"
    return 0
  fi
  xrun config-load 30 "$P/bin/codex" features list
  if [ "$XRC" -eq 0 ]; then CK_STATUS=pass; CK_DETAIL="codex features list exited 0"
  else CK_STATUS=fail; CK_DETAIL="codex features list exited $XRC: $(tail_of "$LOGS/config-load.err")"; fi
}

# ---- tier-0 helpers (§D3)

# t0_py CMD ARGS...: small JSON/capture readers for the checks below; prints a result, exit 0 unless noted.
t0_py() {
  python3 - "$@" <<'PY'
import glob, json, os, re, sys

cmd, args = sys.argv[1], sys.argv[2:]


def load(path):
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def owned_groups(doc, event):
    hooks = doc.get("hooks") if isinstance(doc, dict) else None
    groups = hooks.get(event) if isinstance(hooks, dict) else None
    out = []
    for g in groups if isinstance(groups, list) else []:
        cmds = [h.get("command", "") for h in g.get("hooks", []) if isinstance(h, dict)] if isinstance(g, dict) else []
        if any("herdr-threads-owner:" in c for c in cmds):
            out.append(g)
    return out


def hook_captures(harness, capdir):
    """(stdin path, parsed stdin or None) of every capture whose argv ends `hook <harness>` or
    `hook <harness> --event <NAME>` (setup registers each hook with its event)."""
    found = []
    for argv_path in sorted(glob.glob(os.path.join(capdir, "*.argv"))):
        try:
            argv = open(argv_path, encoding="utf-8", errors="replace").read().splitlines()
        except OSError:
            continue
        evented = (
            len(argv) >= 4
            and argv[-4:-2] == ["hook", harness]
            and argv[-2] == "--event"
            and re.fullmatch(r"[A-Za-z0-9]{1,63}", argv[-1]) is not None
        )
        if argv[-2:] != ["hook", harness] and not evented:
            continue
        stdin_path = argv_path[: -len(".argv")] + ".stdin"
        try:
            doc = json.loads(open(stdin_path, encoding="utf-8", errors="replace").read())
        except (OSError, ValueError):
            doc = None
        found.append((stdin_path, doc))
    return found


if cmd == "setup-assert":  # HARNESS VERSION OUTFILE CONFIGDIR -> `pass|fail<TAB>detail`
    harness, version, outfile, confdir = args
    doc = load(outfile)
    setup = doc.get("setup") if isinstance(doc, dict) else None
    if not isinstance(setup, dict):
        print("fail\tsetup printed no JSON `setup` object")
    elif setup.get("action") != "installed":
        print(f"fail\tsetup.action is {setup.get('action')!r}, expected 'installed'")
    elif (setup.get("harness_version") or {}).get("version") != version:
        print(f"fail\tsetup.harness_version.version is {(setup.get('harness_version') or {}).get('version')!r}, expected {version!r}")
    elif harness == "claude":
        settings = load(os.path.join(confdir, "settings.json"))
        pre = [g for g in owned_groups(settings, "PreToolUse") if "Bash" in str(g.get("matcher", ""))]
        if not owned_groups(settings, "SessionStart"):
            print("fail\tclaude settings.json has no owned SessionStart group")
        elif not pre:
            print("fail\tclaude settings.json has no owned PreToolUse(Bash) group")
        else:
            print("pass\tinstalled; owned SessionStart and PreToolUse(Bash) groups present")
    else:
        hooks = load(os.path.join(confdir, "hooks.json"))
        missing = [e for e in ("SessionStart", "SubagentStart", "PreToolUse") if not owned_groups(hooks, e)]
        if missing:
            print(f"fail\tcodex hooks.json has no owned group for {', '.join(missing)}")
        else:
            print("pass\tinstalled; owned SessionStart, SubagentStart and PreToolUse groups present")
elif cmd == "unsetup-assert":  # HARNESS OUTFILE CONFIGDIR PRECONFIG -> `pass|fail<TAB>detail`
    harness, outfile, confdir, pre = args
    doc = load(outfile)
    setup = doc.get("setup") if isinstance(doc, dict) else None
    if not isinstance(setup, dict):
        print("fail\tunsetup printed no JSON `setup` object")
    elif setup.get("action") != "removed":
        print(f"fail\tsetup.action is {setup.get('action')!r}, expected 'removed'")
    elif harness == "claude":
        if os.path.exists(os.path.join(confdir, "settings.json")):
            print("fail\tclaude settings.json still exists (setup created it, unsetup must delete it)")
        else:
            print("pass\tremoved; settings.json absent again")
    else:
        hooks_path = os.path.join(confdir, "hooks.json")
        try:
            residue = "herdr-threads-owner:" in open(hooks_path, encoding="utf-8", errors="replace").read()
        except OSError:
            residue = False
        conf = os.path.join(confdir, "config.toml")
        before = open(pre, "rb").read() if os.path.exists(pre) else None
        after = open(conf, "rb").read() if os.path.exists(conf) else None
        if residue:
            print("fail\tcodex hooks.json still holds an herdr-threads-owner: command")
        elif before != after:
            print("fail\tcodex config.toml differs from its pre-setup bytes (" + ("absent" if before is None else "present") + " before, " + ("absent" if after is None else "present") + " after)")
        else:
            print("pass\tremoved; hooks.json clean and config.toml equals its pre-setup bytes")
elif cmd == "hook-captured":  # HARNESS CAPDIR -> exit 0 when a SessionStart startup hook capture exists
    for _, doc in hook_captures(args[0], args[1]):
        if isinstance(doc, dict) and doc.get("hook_event_name") == "SessionStart" and doc.get("source") == "startup":
            sys.exit(0)
    sys.exit(1)
elif cmd == "hook-capture-count":  # HARNESS CAPDIR
    print(len(hook_captures(args[0], args[1])))
elif cmd == "rust-schema":  # FILE -> schema result or empty
    doc = load(args[0])
    schema = doc.get("schema") if isinstance(doc, dict) else None
    print(schema.get("result", "") if isinstance(schema, dict) else "")
elif cmd == "rust-schema-detail":  # FILE
    doc = load(args[0])
    schema = (doc or {}).get("schema") if isinstance(doc, dict) else None
    schema = schema if isinstance(schema, dict) else {}
    print(" ".join(f"{k}={schema[k]}" for k in ("fingerprint", "recipe", "reason") if schema.get(k)))
elif cmd == "rust-tier0":  # FILE -> `<observation> <tier-0 payloads parsed>`
    doc = load(args[0])
    doc = doc if isinstance(doc, dict) else {}
    n = sum(1 for p in doc.get("payloads", []) if str(p.get("file", "")).startswith("capture/tier0/"))
    print(str(doc.get("observation", "missing")).split(":")[0], n)
elif cmd == "rust-launch-tables":  # FILE -> `none` or `ok|drift<TAB>detail`
    doc = load(args[0])
    lt = doc.get("launch_tables") if isinstance(doc, dict) else None
    if not isinstance(lt, dict):
        print("none")
    elif lt.get("drift"):
        parts = [f"{k}: {', '.join(lt.get(k) or [])}" for k in
                 ("unknown_subcommands", "unknown_exec_subcommands", "unknown_value_options") if lt.get(k)]
        print("drift\t" + "; ".join(parts))
    else:
        print("ok\tevery subcommand/alias and value option in the help is known to launch.rs")
elif cmd == "admission-of":  # FILE HARNESS
    doc = load(args[0])
    try:
        value = doc["doctor"]["hooks"][args[1]]["installed"]["admission"]
    except (KeyError, TypeError):
        value = ""
    print(value if isinstance(value, str) else "")
else:
    raise SystemExit(f"unknown t0_py command {cmd}")
PY
}

ht_available() { [ -x "$P/ht/herdr-threads" ]; }
# ht_s: the global options every herdr-threads call carries (§D3 `S`): the probe's state directory and a host
# endpoint that never exists (the canary never starts or needs Herdr, §Non-goals).
ht_s() { HT_S=(--state-dir "$P/state" --host-endpoint "$P/herdr.sock" --json); }
# ht_xrun LABEL TIMEOUT SUBCOMMAND ARGS...: herdr-threads under test under the isolated environment.
ht_xrun() {
  local label=$1 timeout=$2; shift 2
  ht_s
  xrun "$label" "$timeout" "$P/ht/herdr-threads" "$@" "${HT_S[@]}"
}

# witness_swap / witness_restore (§D3 t0.hook-fires): while swapped, P/ht/herdr-threads is witness.sh and the real
# binary sits beside it; restore is idempotent and runs before every check that must not be captured.
witness_swap() {
  mv "$P/ht/herdr-threads" "$P/ht/herdr-threads.real" \
    && cp "$CANARY/witness.sh" "$P/ht/herdr-threads" && chmod +x "$P/ht/herdr-threads"
}
witness_restore() {
  if [ -e "$P/ht/herdr-threads.real" ]; then
    rm -f "$P/ht/herdr-threads"
    mv "$P/ht/herdr-threads.real" "$P/ht/herdr-threads"
  fi
}

# status_detail_from LINE: sets CK_STATUS/CK_DETAIL from a `status<TAB>detail` helper line.
status_detail_from() { CK_STATUS=${1%%$'\t'*}; CK_DETAIL=${1#*$'\t'}; }

chk_t0_setup() {
  if ! ht_available; then CK_STATUS=fail; CK_DETAIL="herdr-threads under test is unavailable"; INFRA=1; return 0; fi
  local conf=$P/home/.codex/config.toml
  rm -f "$P/pre-config.toml"
  if [ -e "$conf" ]; then cp "$conf" "$P/pre-config.toml"; fi
  ht_xrun setup 60 setup "$H" --harness-binary "$P/bin/$H"
  if [ "$XRC" -ne 0 ]; then
    CK_STATUS=fail; CK_DETAIL="setup $H exited $XRC: $(tail_of "$LOGS/setup.err") $(tail_of "$LOGS/setup.out")"
    return 0
  fi
  local confdir=$P/home/.claude
  if [ "$H" = codex ]; then confdir=$P/home/.codex; fi
  status_detail_from "$(t0_py setup-assert "$H" "$V" "$LOGS/setup.out" "$confdir")"
}

chk_t0_hook_fires() {
  HOOK_FIRES_RAN=1
  if ! ht_available; then CK_STATUS=fail; CK_DETAIL="herdr-threads under test is unavailable"; INFRA=1; return 0; fi
  if ! witness_swap; then
    witness_restore
    CK_STATUS=fail; CK_DETAIL="could not install the witness at $P/ht/herdr-threads"; INFRA=1; return 0
  fi
  rm -f "$P/stub.stop" "$P/hook-fired" "$P/hook-poll.stop"
  local py3; py3=$(command -v python3)
  python3 "$RUN_PY" --timeout 120 --until-file "$P/stub.stop" -- "$py3" "$CANARY/stub_api.py" --lifetime 120 \
    >"$LOGS/stub.out" 2>"$LOGS/stub.err" </dev/null &
  local stub=$! port=''
  for _ in $(seq 1 100); do
    port=$(head -n 1 "$LOGS/stub.out" 2>/dev/null || true)
    [[ $port =~ ^[0-9]+$ ]] && break
    port=''; sleep 0.1
  done
  if [ -z "$port" ]; then
    : > "$P/stub.stop"; wait "$stub" 2>/dev/null || true; witness_restore
    CK_STATUS=fail; CK_DETAIL="the local 401 stub printed no port: $(tail_of "$LOGS/stub.err")"; INFRA=1; return 0
  fi
  local hf_env=() cmd=()
  if [ "$H" = claude ]; then
    hf_env=("ANTHROPIC_BASE_URL=http://127.0.0.1:$port" ANTHROPIC_API_KEY=canary-invalid CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1)
    cmd=("$P/bin/claude" -p canary)
  else
    cmd=("$P/bin/codex" --no-daemon exec --ephemeral --dangerously-bypass-hook-trust --skip-git-repo-check -s read-only
      -c "model_providers.canary={name=\"canary\",base_url=\"http://127.0.0.1:$port/v1\",wire_api=\"responses\"}"
      -c model_provider=canary canary)
  fi
  # The poller ends the run (via the until-file) as soon as the SessionStart startup capture appears.
  (
    while [ ! -e "$P/hook-poll.stop" ]; do
      if t0_py hook-captured "$H" "$P/capture/tier0"; then : > "$P/hook-fired"; break; fi
      sleep 0.2
    done
  ) &
  local poller=$! started; started=$(date +%s)
  XRC=0
  # shellcheck disable=SC2016  # $1 expands in the inner sh
  python3 "$RUN_PY" --timeout 30 --until-file "$P/hook-fired" -- "${ENVV[@]}" ${hf_env[@]+"${hf_env[@]}"} \
    sh -c 'cd "$1" || exit 127; shift; exec "$@"' sh "$P/proj" "${cmd[@]}" \
    >"$LOGS/hook-fires.out" 2>"$LOGS/hook-fires.err" </dev/null || XRC=$?
  : > "$P/hook-poll.stop"; wait "$poller" 2>/dev/null || true
  : > "$P/stub.stop"; wait "$stub" 2>/dev/null || true
  witness_restore
  local elapsed=$(( $(date +%s) - started ))
  if t0_py hook-captured "$H" "$P/capture/tier0"; then
    CK_STATUS=pass; CK_DETAIL="SessionStart startup captured within ${elapsed}s (no network key; local 401 stub)"
  else
    CK_STATUS=fail
    CK_DETAIL="no SessionStart startup capture for hook $H within 30 s (run exited $XRC after ${elapsed}s): $(tail_of "$LOGS/hook-fires.err") $(tail_of "$LOGS/hook-fires.out")"
  fi
}

chk_t0_payload_parse() {
  # A hook-fires run that left no `hook <h>` capture has nothing for the parse to prove (coverage r2): fail
  # here rather than pass vacuously over an empty capture/tier0.
  local captured=0
  if [ "${HOOK_FIRES_RAN:-0}" = 1 ]; then
    captured=$(t0_py hook-capture-count "$H" "$P/capture/tier0")
    if [ "$captured" -eq 0 ]; then
      CK_STATUS=fail; CK_DETAIL="t0.hook-fires ran but capture/tier0 holds no capture whose argv ends 'hook $H' (optionally followed by --event NAME)"
      return 0
    fi
  fi
  if [ -z "$CARGO_BIN" ]; then CK_STATUS=fail; CK_DETAIL="cargo not found"; return 0; fi
  local cargo_home=${CARGO_HOME:-$REAL_HOME/.cargo} rustup_home=${RUSTUP_HOME:-$REAL_HOME/.rustup}
  local target=${CARGO_TARGET_DIR:-$ROOT/target}
  local e=(env -i "HOME=$P/home" "CLAUDE_CONFIG_DIR=$P/home/.claude" "CODEX_HOME=$P/home/.codex"
    "XDG_CONFIG_HOME=$P/home/.config" "XDG_STATE_HOME=$P/home/.local/state"
    "XDG_DATA_HOME=$P/home/.local/share" "XDG_CACHE_HOME=$P/home/.cache" "TMPDIR=$P/tmp"
    "PATH=$cargo_home/bin:$(dirname "$CARGO_BIN"):/usr/bin:/bin" "LANG=C.UTF-8" "TERM=dumb"
    "CARGO_HOME=$cargo_home" "RUSTUP_HOME=$rustup_home" "CARGO_TARGET_DIR=$target"
    "HT_CANARY_CAPTURE_DIR=$P")
  if [ -n "${RUSTUP_TOOLCHAIN:-}" ]; then e+=("RUSTUP_TOOLCHAIN=$RUSTUP_TOOLCHAIN"); fi
  if [ -n "$VERSIONS_JSON" ]; then e+=("HT_TEST_RECIPES_JSON=$VERSIONS_JSON"); fi
  XRC=0
  (cd "$ROOT" && python3 "$RUN_PY" --timeout 1800 -- "${e[@]}" nice cargo test --locked --all-features --lib \
    canary_payloads -- --nocapture >"$LOGS/payload-parse.out" 2>"$LOGS/payload-parse.err" </dev/null) || XRC=$?
  if [ "$XRC" -eq 0 ]; then
    local obs parsed
    read -r obs parsed <<<"$(t0_py rust-tier0 "$P/canary-rust.json")"
    if [ "${HOOK_FIRES_RAN:-0}" = 1 ] && [ "$obs" = ok ] && [ "$parsed" -eq 0 ]; then
      CK_STATUS=fail; CK_DETAIL="canary_payloads parsed no tier-0 payload although $captured hook capture(s) exist"
    else
      CK_STATUS=pass; CK_DETAIL="canary_payloads parsed $parsed tier-0 payload(s) ($captured hook capture(s))"
    fi
  elif grep -q 'test result:' "$LOGS/payload-parse.out"; then
    CK_STATUS=fail; CK_DETAIL="canary_payloads failed: $(grep -h -A3 'canary payload failures' "$LOGS/payload-parse.out" | tr '\n\t' '  ' | head -c 400)"
  else
    CK_STATUS=fail; CK_DETAIL="canary_payloads did not run (exit $XRC): $(tail_of "$LOGS/payload-parse.err")"
    INFRA=1
  fi
}

chk_t0_schema() {
  if [ "$H" != codex ]; then CK_STATUS=skip; CK_DETAIL="claude has no embedded-schema fingerprint"; return 0; fi
  SCHEMA_RESULT=$(t0_py rust-schema "$P/canary-rust.json")
  local detail; detail=$(t0_py rust-schema-detail "$P/canary-rust.json")
  case $SCHEMA_RESULT in
    match) CK_STATUS=pass; CK_DETAIL="fingerprint equals a recipe's: $detail" ;;
    drift) CK_STATUS=warn; CK_DETAIL="fingerprint differs from every recipe (an adapter may be needed): $detail" ;;
    unextractable) CK_STATUS=warn; CK_DETAIL="no fingerprint could be taken: $detail" ;;
    *) SCHEMA_RESULT=''; CK_STATUS=warn; CK_DETAIL="canary-rust.json has no schema result (t0.payload-parse did not complete)" ;;
  esac
}

chk_t0_admission() {
  witness_restore
  if ! ht_available; then CK_STATUS=fail; CK_DETAIL="herdr-threads under test is unavailable"; INFRA=1; return 0; fi
  ht_xrun doctor 60 doctor
  # doctor exits non-zero while no daemon runs (the canary never starts one); only its JSON matters
  local line status detail
  line=$(python3 "$CANARY/admission.py" evaluate --harness "$H" --expected "$EXPECTED_ADMISSION" \
    ${SCHEMA_RESULT:+--schema "$SCHEMA_RESULT"} "$LOGS/doctor.out")
  IFS=$'\t' read -r status detail ADMISSION_OBSERVED <<<"$line"
  case $status in
    pass|fail|skip) CK_STATUS=$status; CK_DETAIL=$detail ;;
    infra) CK_STATUS=fail; CK_DETAIL=$detail; INFRA=1 ;;
    *) CK_STATUS=fail; CK_DETAIL="admission.py printed no verdict: $line" ;;
  esac
  if [ "$CK_STATUS" = fail ] && [ "$XRC" -ne 0 ] && [ "$XRC" -ne 3 ] && [ -z "$ADMISSION_OBSERVED" ]; then
    CK_DETAIL="$CK_DETAIL (doctor exited $XRC: $(tail_of "$LOGS/doctor.err"))"
  fi
}

chk_t0_launch_flags() {
  local tsv=$CANARY/launch-flags.tsv n=0 missing='' row_harness cmd flag source words label
  if [ ! -f "$tsv" ]; then CK_STATUS=fail; CK_DETAIL="$tsv is missing"; return 0; fi
  while IFS=$'\t' read -r row_harness cmd flag source; do
    case $row_harness in ''|'#'*) continue ;; esac
    [ "$row_harness" = "$H" ] || continue
    n=$((n + 1)); label=launch-flags-$n
    read -r -a words <<<"$cmd"
    words[0]=$P/bin/$H
    xrun "$label" 30 "${words[@]}"
    cat "$LOGS/$label.out" "$LOGS/$label.err" > "$LOGS/$label.txt"
    if ! grep -Eq -- "(^|[^[:alnum:]-])$flag([^[:alnum:]-]|\$)" "$LOGS/$label.txt"; then
      missing+="${missing:+; }'$flag' not in '$cmd' (exit $XRC; $source)"
    fi
  done < "$tsv"
  if [ "$n" -eq 0 ]; then CK_STATUS=fail; CK_DETAIL="launch-flags.tsv has no row for $H"
  elif [ -n "$missing" ]; then CK_STATUS=fail; CK_DETAIL="$missing"
  else CK_STATUS=pass; CK_DETAIL="all $n launch flag(s) present in the help output"; fi
}

chk_t0_npm_shim_admission() {
  if [ "$H" != codex ]; then CK_STATUS=skip; CK_DETAIL="codex only (the npm codex.js shim)"; return 0; fi
  witness_restore
  if [ ! -e "$P/npm/node_modules/.bin/codex" ] || ! ht_available; then
    CK_STATUS=warn; CK_DETAIL="no npm .bin/codex shim or no herdr-threads to ask"; return 0
  fi
  ht_s
  # a later PATH assignment wins: only the shim directory moves in front
  xrun doctor-shim 60 env "PATH=$P/npm/node_modules/.bin:$P/bin:$NODE_DIR:/usr/bin:/bin" "$P/ht/herdr-threads" doctor "${HT_S[@]}"
  local shim; shim=$(t0_py admission-of "$LOGS/doctor-shim.out" "$H")
  if [ -z "$shim" ]; then CK_STATUS=warn; CK_DETAIL="doctor with the npm shim first on PATH printed no admission (exit $XRC)"
  elif [ "$shim" != "${ADMISSION_OBSERVED:-}" ]; then
    CK_STATUS=warn; CK_DETAIL="with the npm shim first on PATH admission is '$shim', with the native binary '${ADMISSION_OBSERVED:-unknown}' (E4)"
  else CK_STATUS=pass; CK_DETAIL="admission '$shim' either way"; fi
}

chk_t0_launch_tables() {
  if [ "$H" != codex ]; then CK_STATUS=skip; CK_DETAIL="codex only"; return 0; fi
  local line; line=$(t0_py rust-launch-tables "$P/canary-rust.json")
  case $line in
    none) CK_STATUS=warn; CK_DETAIL="canary-rust.json has no launch_tables result (no help texts or t0.payload-parse did not complete)" ;;
    drift*) CK_STATUS=warn; CK_DETAIL="launch.rs tables lag codex --help: ${line#*$'\t'}" ;;
    *) CK_STATUS=pass; CK_DETAIL=${line#*$'\t'} ;;
  esac
}

chk_t0_unsetup() {
  witness_restore
  if ! ht_available; then CK_STATUS=fail; CK_DETAIL="herdr-threads under test is unavailable"; INFRA=1; return 0; fi
  ht_xrun unsetup 60 unsetup "$H"
  if [ "$XRC" -ne 0 ]; then
    CK_STATUS=fail; CK_DETAIL="unsetup $H exited $XRC: $(tail_of "$LOGS/unsetup.err") $(tail_of "$LOGS/unsetup.out")"
    return 0
  fi
  local confdir=$P/home/.claude
  if [ "$H" = codex ]; then confdir=$P/home/.codex; fi
  status_detail_from "$(t0_py unsetup-assert "$H" "$LOGS/unsetup.out" "$confdir" "$P/pre-config.toml")"
}

chk_t0_planted() {
  local want=${HT_CANARY_PLANT_BREAK#*@} wh=${HT_CANARY_PLANT_BREAK%%@*}
  if [ "$wh" = "$H" ] && version_ge "$V" "$want"; then
    CK_STATUS=fail; CK_DETAIL="planted break at $V (>= $want)"
  else
    CK_STATUS=pass; CK_DETAIL="below planted version $want or other harness"
  fi
}

chk_t0_isolation() {
  local detail rc=0
  detail=$(python3 "$CANARY/isolation.py" check "$P/isolation-before.json" 2>&1) || rc=$?
  if [ "$rc" -eq 0 ]; then CK_STATUS=pass; CK_DETAIL=$detail
  else CK_STATUS=fail; CK_DETAIL=$detail; INFRA=1; fi
}

# run_tier1: the tier-1 call site (§D6). The planted failure and the key gate come first, so a planted or
# skipped probe returns before tier1.sh is even sourced.
run_tier1() {
  local id
  if [ -n "${HT_CANARY_PLANT_TIER1_FAIL:-}" ]; then
    local spec=${HT_CANARY_PLANT_TIER1_FAIL%%:*} count='' done_n=0
    [[ $HT_CANARY_PLANT_TIER1_FAIL != *:* ]] || count=${HT_CANARY_PLANT_TIER1_FAIL#*:}
    [[ $spec == *@* ]] && [[ -z $count || $count =~ ^[0-9]+$ ]] \
      || die "HT_CANARY_PLANT_TIER1_FAIL must be <harness>@<version>[:count]"
    if [ "$MODEL_TIER" != off ] && [ "$spec" = "$H@$V" ]; then
      local counter=$ATTEMPT_FILE_DIR/planted-tier1-$H-$V
      [ ! -f "$counter" ] || done_n=$(cat "$counter")
      if [ -z "$count" ] || [ "$done_n" -lt "$count" ]; then
        echo $((done_n + 1)) > "$counter"
        record_check t1.planted fail "planted tier-1 failure for $H@$V (attempt $((done_n + 1))${count:+ of $count})"
        return 0
      fi
      record_check t1.planted pass "planted tier-1 failure exhausted after $count attempt(s)"
    fi
  fi
  local detail=
  if [ "$MODEL_TIER" = off ]; then detail="--model-tier off"
  elif ! tier1_enabled "$H"; then detail="$(tier1_keyvar "$H") is empty"; fi
  if [ -n "$detail" ]; then
    for id in session-start pre-tool-use context-delivery payload-parse; do record_check "t1.$id" skip "$detail"; done
    return 0
  fi
  # shellcheck source=canary/tier1.sh
  . "$CANARY/tier1.sh"
  tier1_run "$(tier1_key "$H")" "$(tier1_keyvar "$H")"
}

record_check() { # id status detail -> $P/checks.tsv
  local d=${3//$'\n'/ }; d=${d//$'\t'/ }
  printf '%s\t%s\t%s\n' "$1" "$2" "$d" >> "$P/checks.tsv"
}

run_check() {
  local id=$1 fn
  fn=chk_${id//[.-]/_}
  CK_STATUS=fail; CK_DETAIL="check function $fn is missing"
  if declare -F "$fn" >/dev/null; then "$fn" || true; fi
  record_check "$id" "$CK_STATUS" "$CK_DETAIL"
}

# ---------------------------------------------------------------- one probe (§D2, §D3)

ATTEMPT_FILE_DIR=$OUT/work

probe_finish() { # forced-result -> prints the probeResult, returns its exit code via PROBE_RC
  local forced=$1 doc
  doc=$(canary_py probe-json "$P/checks.tsv" "$forced" "$P/canary-rust.json" "$P/canary-rust-tier1.json" 2>"$P/rc.txt") || { PROBE_RC=2; echo '{"result":"infra","checks":[],"failed_tier":null}'; return 0; }
  PROBE_RC=$(cat "$P/rc.txt")
  printf '%s\n' "$doc"
}

probe_one() {
  H=$1; V=$2
  case $H in claude|codex) ;; *) die "unknown harness $H" ;; esac
  [[ $V =~ ^(0|[1-9][0-9]{0,5})\.(0|[1-9][0-9]{0,5})\.(0|[1-9][0-9]{0,5})$ ]] || die "version must be X.Y.Z: $V"
  local n=1
  while [ -e "$ATTEMPT_FILE_DIR/$H-$V-$n" ]; do n=$((n + 1)); done
  P=$ATTEMPT_FILE_DIR/$H-$V-$n
  LOGS=$P/logs
  mkdir -p "$P/proj" "$P/state" "$P/ht" "$P/capture/tier0" "$P/capture/tier1" "$P/help" "$LOGS"
  : > "$P/checks.tsv"
  INFRA=0
  HOOK_FIRES_RAN=0; SCHEMA_RESULT=''; ADMISSION_OBSERVED=''; EXPECTED_ADMISSION=unasserted
  build_env "$P"
  python3 "$CANARY/isolation.py" snapshot "$P/isolation-before.json"

  local pkg
  if [ "$H" = claude ]; then pkg=@anthropic-ai/claude-code; else pkg=@openai/codex; fi
  local install_rc=0
  XRC=0
  python3 "$RUN_PY" --timeout 900 -- "${ENVV[@]}" "npm_config_cache=$OUT/npm-cache" npm_config_update_notifier=false \
    "$NPM_BIN" install --prefix "$P/npm" --no-audit --no-fund --no-save "$pkg@$V" \
    >"$LOGS/npm-install.out" 2>"$LOGS/npm-install.err" </dev/null || install_rc=$?
  local bin=
  if [ "$install_rc" -ne 0 ]; then
    record_check t0.install fail "npm install $pkg@$V exited $install_rc: $(tail_of "$LOGS/npm-install.err")"
    probe_finish infra
  else
    if [ "$H" = claude ]; then
      bin=$P/npm/node_modules/@anthropic-ai/claude-code/bin/claude.exe
      if [ ! -x "$bin" ] || ! "${ENVV[@]}" "$bin" --version >/dev/null 2>&1; then
        # E2: the package ships a placeholder until `node install.cjs` copies the native binary in.
        # shellcheck disable=SC2016  # $1 expands in the inner sh
        xrun install-cjs 300 sh -c 'cd "$1" && exec node install.cjs' sh "$P/npm/node_modules/@anthropic-ai/claude-code"
      fi
    else
      set -- "$P"/npm/node_modules/@openai/codex-*/vendor/*/bin/codex
      if [ $# -eq 1 ] && [ -e "$1" ]; then bin=$1; fi
    fi
    if [ -z "$bin" ] || [ ! -e "$bin" ]; then
      record_check t0.install fail "no $H binary found under $P/npm after install"
      probe_finish infra
    else
      ln -s "$bin" "$P/bin/$H"
      EXPECTED_ADMISSION=$(canary_py expected "$H" "$V" "$VJSON") || EXPECTED_ADMISSION=unasserted
      write_canary_probe "$P" "$H" "$V" "$P/bin/$H" "$EXPECTED_ADMISSION"
      if [ "$H" = codex ]; then
        # help texts for launch-tables (§D5), produced under the isolated environment
        xrun help-codex 30 "$P/bin/codex" --help
        cat "$LOGS/help-codex.out" "$LOGS/help-codex.err" > "$P/help/codex.txt"
        xrun help-codex-exec 30 "$P/bin/codex" exec --help
        cat "$LOGS/help-codex-exec.out" "$LOGS/help-codex-exec.err" > "$P/help/codex-exec.txt"
      fi
      if ensure_ht; then
        cp "$HT_BIN" "$P/ht/herdr-threads"
      else
        echo "harness-canary.sh: warning: herdr-threads under test is unavailable; tier-0 checks that need it will fail" >&2
      fi
      local id
      for id in "${TIER0_CHECKS[@]}"; do run_check "$id"; done
      run_tier1
      run_check t0.isolation
      if [ "$INFRA" -eq 1 ]; then probe_finish infra; else probe_finish auto; fi
    fi
  fi
  [ "$KEEP" -eq 1 ] || rm -rf "$P"
}

# shellcheck disable=SC2317  # `exit` is reached when the file is executed rather than sourced
if [ -n "${HT_CANARY_SOURCE_ONLY:-}" ]; then return 0 2>/dev/null || exit 0; fi

if [ "$MODE" = probe ]; then
  P=; LOGS=; PROBE_RC=2
  probe_one "$PROBE_HARNESS" "$PROBE_VERSION"
  [ "$OUT_IS_TEMP" -eq 0 ] || [ "$KEEP" -eq 1 ] || rm -rf "$OUT"
  exit "$PROBE_RC"
fi

# ---------------------------------------------------------------- orchestration (§D1, §D7, §D8)

case $HARNESS in both) HARNESSES=claude,codex ;; *) HARNESSES=$HARNESS ;; esac

npm_versions() { # harness -> JSON list file; 3 attempts, 60 s each (§D1)
  local pkg=$1 file=$2
  [ "$pkg" = claude ] && pkg=@anthropic-ai/claude-code || pkg=@openai/codex
  P=$OUT/work/_view; LOGS=$P/logs; mkdir -p "$LOGS"
  build_env "$P"
  for _ in 1 2 3; do
    XRC=0
    python3 "$RUN_PY" --timeout 60 -- "${ENVV[@]}" "npm_config_cache=$OUT/npm-cache" npm_config_update_notifier=false \
      "$NPM_BIN" view "$pkg" versions --json >"$file" 2>"$LOGS/npm-view.err" </dev/null || XRC=$?
    if [ "$XRC" -eq 0 ] && [ -s "$file" ]; then return 0; fi
  done
  return 1
}

explicit=
case $VERSIONS in latest|since-verified) ;; *) explicit=$VERSIONS ;; esac

ensure_ht || echo "harness-canary.sh: warning: could not build herdr-threads; probes will report it" >&2

# main's per-harness contract ids ({"claude": ..., "codex": ...}); selection is scoped to them.
MAIN_IDS=
if [ -n "$HT_BIN" ] && MAIN_IDS=$("$HT_BIN" contract-id --json 2>/dev/null) && [ -n "$MAIN_IDS" ]; then :; else
  MAIN_IDS=
  echo "harness-canary.sh: warning: no contract id (herdr-threads unavailable); selection is contract-agnostic and nothing is re-probed" >&2
fi
main_id_for() {
  [ -n "$MAIN_IDS" ] || return 0
  MAIN_IDS_JSON=$MAIN_IDS python3 -c 'import json,os,sys; print(json.loads(os.environ["MAIN_IDS_JSON"]).get(sys.argv[1]) or "")' "$1"
}

for h in ${HARNESSES//,/ }; do
  listfile=$OUT/work/npm-$h-versions.json
  npm_versions "$h" "$listfile" || { echo "harness-canary.sh: npm view failed for $h (see $OUT/work/_view/logs)" >&2; exit 2; }
  mode=$VERSIONS; [ -z "$explicit" ] || mode=list
  mid=$(main_id_for "$h") || exit 2
  cands=$(canary_py candidates "$h" "$mode" "$BJSON" "$explicit" "$listfile" "$mid") || exit 2
  vmax=$(canary_py verified-max "$h" "$BJSON" "$mid") || exit 2
  base=${BASELINE:-${vmax:-0.0.0}}
  kb=$(canary_py known-broken "$h" "$BJSON" "$mid") || exit 2
  KEEP_FLAG=(); if [ "$KEEP" -eq 1 ]; then KEEP_FLAG=(--keep); fi
  probe_cmd=$(printf '%q ' "$SCRIPT_PATH" --probe "$h" '{version}' --out "$OUT" --model-tier "$MODEL_TIER" \
    ${HT_BIN:+--herdr-threads "$HT_BIN"} ${VERSIONS_JSON:+--versions-json "$VERSIONS_JSON"} \
    ${KEEP_FLAG[@]+"${KEEP_FLAG[@]}"})
  bisect_args=(--probe-cmd "$probe_cmd" --candidates "$cands" --baseline "$base" --known-broken "$kb")
  if [ "$BISECT" -eq 1 ]; then bisect_args+=(--bisect); fi
  if tier1_enabled "$h"; then bisect_args+=(--tier1); fi
  rp=
  if [ "$mode" = since-verified ]; then rp=$(canary_py reprobe "$h" "$BJSON" "$listfile" "$mid") || exit 2; fi
  if [ -n "$rp" ]; then
    bisect_args+=(--reprobe "$rp")
    echo "harness-canary.sh: $h re-probing versions known_broken under another contract: $rp" >&2
  fi
  echo "harness-canary.sh: $h candidates: ${cands:-none} (baseline $base)" >&2
  python3 "$CANARY/bisect.py" "${bisect_args[@]}" > "$OUT/bisect-$h.json" || true
  [ -s "$OUT/bisect-$h.json" ] || { echo "harness-canary.sh: bisect produced no result for $h" >&2; exit 2; }
done

inputs=$(printf '{"harness":%s,"versions":%s,"bisect":%s,"model_tier":%s}' "$(json_str "$HARNESS")" \
  "$(json_str "$VERSIONS")" "$([ "$BISECT" -eq 1 ] && echo true || echo false)" "$(json_str "$MODEL_TIER")")
commit=$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo unknown)
htv=$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -n 1)
code=$(canary_py report "$OUT" "$HARNESSES" "$inputs" "$commit" "${htv:-unknown}" \
  "$(uname -s | tr '[:upper:]' '[:lower:]')" "$(uname -m)" "$BJSON" "$BASELINE" "$MAIN_IDS") || exit 2
echo "harness-canary.sh: report in $OUT/canary-report.json and $OUT/summary.md (exit $code)" >&2
if [ "$KEEP" -ne 1 ]; then rm -rf "$OUT/work" "$OUT/npm-cache"; fi
exit "$code"
