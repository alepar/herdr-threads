#!/usr/bin/env python3
"""Owned claude canary backend; generic runner supplies validated isolated inputs."""
import json
import importlib.util
import pathlib
import sys

SHELL_FUNCTIONS = r"""KEY_CANARY_NATIVE=${ANTHROPIC_API_KEY:-}
tier1_keyvar() { echo ANTHROPIC_API_KEY; }
tier1_key() { printf '%s' "$KEY_CANARY_NATIVE"; }

probe_one() {
  H=$1; V=$2
  [ "$H" = claude ] || die "wrong companion harness"
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
  pkg=$HT_CANARY_PACKAGE
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
      bin=$P/npm/node_modules/@anthropic-ai/claude-code/bin/claude.exe
      if [ ! -x "$bin" ] || ! "${ENVV[@]}" "$bin" --version >/dev/null 2>&1; then
        # E2: the package ships a placeholder until `node install.cjs` copies the native binary in.
        # shellcheck disable=SC2016  # $1 expands in the inner sh
        xrun install-cjs 300 sh -c 'cd "$1" && exec node install.cjs' sh "$P/npm/node_modules/@anthropic-ai/claude-code"
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

adapter_model_run() {
    python3 - "$P/proj/.claude/settings.local.json" "$hookcmd" <<'PY'
import json, os, sys
os.makedirs(os.path.dirname(sys.argv[1]), exist_ok=True)
hook = {"type": "command", "command": sys.argv[2], "timeout": 10}
json.dump({"hooks": {"SessionStart": [{"hooks": [hook]}],
                     "PreToolUse": [{"matcher": "Bash", "hooks": [hook]}]},
           "permissions": {"allow": ["Bash(echo canary-ok)"]}}, open(sys.argv[1], "w"))
PY
    t1_xrun t1-run 180 "$keyvar" "$T1/key" "$P/proj" "$T1/bin/claude" -p "$T1_PROMPT" \
      --model "${HT_CANARY_CLAUDE_MODEL:-claude-haiku-4-5-20251001}" --max-budget-usd 0.10 \
      --setting-sources project,local --output-format json
    rc=$XRC
}
"""


def final_text(raw):
    try:
        doc = json.loads(raw)
    except ValueError:
        return ""
    items = doc if isinstance(doc, list) else [doc]
    for item in reversed(items):
        if isinstance(item, dict) and isinstance(item.get("result"), str):
            return item["result"]
    return ""


if __name__ == "__main__":
    if sys.argv[1:] == ["--shell-functions"]:
        print(SHELL_FUNCTIONS)
    else:
        spec = importlib.util.spec_from_file_location("canary_run", pathlib.Path(__file__).parents[1] / "run.py")
        runner = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(runner)
        sys.exit(runner.legacy_companion(sys.argv[1:], "claude", pathlib.Path(__file__).resolve()))
