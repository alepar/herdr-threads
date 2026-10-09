#!/usr/bin/env python3
"""Owned codex canary backend; generic runner supplies validated isolated inputs."""
import json
import importlib.util
import pathlib
import sys

SHELL_FUNCTIONS = r"""KEY_CANARY_NATIVE=${OPENAI_API_KEY:-}
tier1_keyvar() { echo OPENAI_API_KEY; }
tier1_key() { printf '%s' "$KEY_CANARY_NATIVE"; }

probe_one() {
  H=$1; V=$2
  [ "$H" = codex ] || die "wrong companion harness"
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
      set -- "$P"/npm/node_modules/@openai/codex-*/vendor/*/bin/codex
      if [ $# -eq 1 ] && [ -e "$1" ]; then bin=$1; fi
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
    # shellcheck disable=SC2016  # $1/$2 expand in the inner sh
    xrun t1-login 60 sh -c 'exec "$1" login --with-api-key < "$2"' sh "$T1/bin/codex" "$T1/key"
    if [ "$XRC" -ne 0 ]; then
      rc=$XRC; note="codex login --with-api-key exited $XRC: $(tail_of "$LOGS/t1-login.err")"
    else
      local esc=${hookcmd//\\/\\\\}
      esc=${esc//\"/\\\"}
      local hs="[{hooks=[{type=\"command\",command=\"$esc\",timeout=10}]}]"
      local hb="[{matcher=\"^Bash\$\",hooks=[{type=\"command\",command=\"$esc\",timeout=10}]}]"
      t1_xrun t1-run 180 - "$T1/key" "$P/proj" "$T1/bin/codex" --no-daemon exec --ephemeral --ignore-user-config \
        --dangerously-bypass-hook-trust --json --skip-git-repo-check -s read-only \
        -m "${HT_CANARY_CODEX_MODEL:-gpt-6-luna}" -c 'model_reasoning_effort="low"' \
        -c "hooks.SessionStart=$hs" -c "hooks.PreToolUse=$hb" "$T1_PROMPT"
      rc=$XRC
    fi
}
"""


def final_text(raw):
    last = ""
    for line in raw.splitlines():
        try:
            ev = json.loads(line)
        except ValueError:
            continue
        item = ev.get("item") if isinstance(ev, dict) else None
        if isinstance(item, dict) and item.get("type") == "agent_message" and isinstance(item.get("text"), str):
            last = item["text"]
    return last


if __name__ == "__main__":
    if sys.argv[1:] == ["--shell-functions"]:
        print(SHELL_FUNCTIONS)
    else:
        spec = importlib.util.spec_from_file_location("canary_run", pathlib.Path(__file__).parents[1] / "run.py")
        runner = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(runner)
        sys.exit(runner.legacy_companion(sys.argv[1:], "codex", pathlib.Path(__file__).resolve()))
