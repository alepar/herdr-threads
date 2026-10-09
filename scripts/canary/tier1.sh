#!/usr/bin/env bash
# tier1.sh — tier 1 (model) of the harness canary (nested spec §D6). Sourced by harness-canary.sh from its
# tier-1 call site (run_tier1), only once the key gate has passed; it defines tier1_run and runs nothing itself.
#
# tier1_run KEY KEYVAR: one model run of harness $H on a FRESH home (no herdr-threads setup) with the capture
# hooks of capture_hook.py supplied per invocation, then the four t1.* checks recorded with record_check.
# The key reaches the harness through a mode-0600 file inside the probe directory, never through argv, and
# the file is deleted as soon as the run ends.
#
# Uses from the caller: P H V LOGS CANARY RUN_PY ROOT NODE_DIR CARGO_BIN REAL_HOME VERSIONS_JSON INFRA ENVV
# build_env xrun record_check tail_of.
# shellcheck shell=bash disable=SC2154,SC2034  # the caller's variables are listed above (INFRA is set for it)

# shellcheck disable=SC2016  # the backticks are literal prompt text
T1_PROMPT='Run the shell command `echo canary-ok`. Then reply with every line that starts with `canary-nonce-` from your context, verbatim, and nothing else.'

# t1_xrun LABEL TIMEOUT KEYVAR KEYFILE CWD CMD...: xrun CMD in CWD; KEYVAR ("-" for none) is exported from KEYFILE.
t1_xrun() {
  local label=$1 timeout=$2 keyvar=$3 keyfile=$4 cwd=$5; shift 5
  # shellcheck disable=SC2016  # the variables expand in the inner sh
  xrun "$label" "$timeout" sh -c 'var=$1; kf=$2; cd "$3" || exit 127; shift 3
    if [ "$var" != - ]; then v=$(cat "$kf") && export "$var=$v"; fi; exec "$@"' sh "$keyvar" "$keyfile" "$cwd" "$@"
}

# t1_eval CAPDIR NONCEFILE OUTFILE HARNESS RC NOTE: prints one `id<TAB>status<TAB>detail` line per t1.* check
# except t1.payload-parse (which needs cargo).
t1_eval() {
  python3 - "$@" <<'PY'
import glob, json, os, sys

capdir, noncefile, outfile, harness, rc, note = sys.argv[1:7]
nonces = json.load(open(noncefile))
recs = []
for p in sorted(glob.glob(os.path.join(capdir, "*.json"))):
    try:
        recs.append(json.load(open(p)))
    except (OSError, ValueError):
        pass


def payload(rec):
    try:
        doc = json.loads(rec.get("stdin", ""))
    except (ValueError, TypeError):
        return {}
    return doc if isinstance(doc, dict) else {}


def command(rec):
    tool_input = payload(rec).get("tool_input")
    return str(tool_input.get("command", "")) if isinstance(tool_input, dict) else ""


# The owned companion interprets its native output format.
import importlib.util
spec = importlib.util.spec_from_file_location("owned_canary", sys.argv[7])
owned = importlib.util.module_from_spec(spec)
spec.loader.exec_module(owned)
final_text = owned.final_text


try:
    text = final_text(open(outfile, encoding="utf-8", errors="replace").read())
except OSError:
    text = ""
suffix = f" ({note})" if note else (f" (model run exited {rc})" if rc != "0" else "")
rows = []
starts = [r for r in recs if r.get("event") == "SessionStart"]
rows.append(("t1.session-start", "pass" if starts else "fail",
             f"{len(starts)} SessionStart payload(s) captured" if starts else "no SessionStart payload captured" + suffix))
pre = [r for r in recs if r.get("event") == "PreToolUse" and "echo canary-ok" in command(r)]
rows.append(("t1.pre-tool-use", "pass" if pre else "fail",
             f"{len(pre)} PreToolUse payload(s) for echo canary-ok" if pre
             else "no PreToolUse payload with command containing 'echo canary-ok'" + suffix))
missing = [e for e in ("SessionStart", "PreToolUse") if nonces[e] not in text]
rows.append(("t1.context-delivery", "fail" if missing else "pass",
             ("final output lacks the " + " and ".join(missing) + " nonce" + suffix) if missing
             else "final output carries both nonces"))
for r in rows:
    print("\t".join(r))
PY
}

tier1_run() {
  local key=$1 keyvar=$2
  local T1=$P/t1 saved=("${ENVV[@]}")
  rm -rf "$T1"
  mkdir -p "$T1" "$P/proj"
  build_env "$T1"  # fresh HOME, CLAUDE_CONFIG_DIR and CODEX_HOME: no herdr-threads setup, no tier-0 state
  ln -s "$P/bin/$H" "$T1/bin/$H"
  (umask 077; printf '%s' "$key" > "$T1/key")
  python3 - "$T1/nonces.json" <<'PY'
import json, secrets, sys
json.dump({"SessionStart": secrets.token_hex(8), "PreToolUse": secrets.token_hex(8)}, open(sys.argv[1], "w"))
PY
  local hookcmd
  hookcmd=$(printf '%q %q %q %q' "$(command -v python3)" "$CANARY/capture_hook.py" "$P/capture/tier1" "$T1/nonces.json")

  local rc=0 note=
  adapter_model_run
  rm -f "$T1/key"
  [ "$rc" -eq 0 ] || [ -n "$note" ] || note="$H exited $rc: $(tail_of "$LOGS/t1-run.err")"

  local out id status detail
  touch "$LOGS/t1-run.out"
  out=$(t1_eval "$P/capture/tier1" "$T1/nonces.json" "$LOGS/t1-run.out" "$H" "$rc" "$note" \
    "${HT_CANARY_COMPANION:-$CANARY/adapters/$H.py}") || out=
  if [ -z "$out" ]; then
    for id in session-start pre-tool-use context-delivery; do record_check "t1.$id" fail "tier-1 evaluation failed"; done
  else
    while IFS=$'\t' read -r id status detail; do record_check "$id" "$status" "$detail"; done <<<"$out"
  fi

  # t1.payload-parse: the gated Rust test over capture/tier1 only (tier-0 payloads are t0.payload-parse's)
  local n; n=$(find "$P/capture/tier1" -type f -name '*.json' 2>/dev/null | wc -l | tr -d ' ')
  if [ "$n" -eq 0 ]; then
    record_check t1.payload-parse skip "nothing captured under capture/tier1"
  elif [ -z "$CARGO_BIN" ]; then
    record_check t1.payload-parse fail "cargo not found"
  else
    mkdir -p "$T1/cap/capture"
    cp "$P/canary-probe.json" "$T1/cap/canary-probe.json"
    ln -s "$P/capture/tier1" "$T1/cap/capture/tier1"
    [ ! -d "$P/help" ] || ln -s "$P/help" "$T1/cap/help"
    local cargo_home=${CARGO_HOME:-$REAL_HOME/.cargo} rustup_home=${RUSTUP_HOME:-$REAL_HOME/.rustup}
    local target=${CARGO_TARGET_DIR:-$ROOT/target}
    local e=(env -i "HOME=$T1/home" "CLAUDE_CONFIG_DIR=$T1/home/.claude" "CODEX_HOME=$T1/home/.codex"
      "XDG_CONFIG_HOME=$T1/home/.config" "XDG_STATE_HOME=$T1/home/.local/state"
      "XDG_DATA_HOME=$T1/home/.local/share" "XDG_CACHE_HOME=$T1/home/.cache" "TMPDIR=$T1/tmp"
      "PATH=$cargo_home/bin:$(dirname "$CARGO_BIN"):/usr/bin:/bin" "LANG=C.UTF-8" "TERM=dumb"
      "CARGO_HOME=$cargo_home" "RUSTUP_HOME=$rustup_home" "CARGO_TARGET_DIR=$target"
      "HT_CANARY_CAPTURE_DIR=$T1/cap")
    if [ -n "${RUSTUP_TOOLCHAIN:-}" ]; then e+=("RUSTUP_TOOLCHAIN=$RUSTUP_TOOLCHAIN"); fi
    if [ -n "$VERSIONS_JSON" ]; then e+=("HT_TEST_RECIPES_JSON=$VERSIONS_JSON"); fi
    XRC=0
    (cd "$ROOT" && python3 "$RUN_PY" --timeout 1800 -- "${e[@]}" nice cargo test --locked --all-features --lib \
      canary_payloads -- --nocapture >"$LOGS/t1-payload-parse.out" 2>"$LOGS/t1-payload-parse.err" </dev/null) || XRC=$?
    # the tier-1 contract verdicts reach the probe result next to the tier-0 ones (harness-canary.sh probe-json)
    if [ -f "$T1/cap/canary-rust.json" ]; then cp "$T1/cap/canary-rust.json" "$P/canary-rust-tier1.json"; fi
    if [ "$XRC" -eq 0 ]; then
      record_check t1.payload-parse pass "canary_payloads parsed $n tier-1 payload(s)"
    elif grep -q 'test result:' "$LOGS/t1-payload-parse.out"; then
      record_check t1.payload-parse fail "canary_payloads failed: $(grep -h -A3 'canary payload failures' "$LOGS/t1-payload-parse.out" | tr '\n\t' '  ' | head -c 400)"
    else
      record_check t1.payload-parse fail "canary_payloads did not run (exit $XRC): $(tail_of "$LOGS/t1-payload-parse.err")"
      INFRA=1
    fi
  fi
  ENVV=("${saved[@]}")
}
