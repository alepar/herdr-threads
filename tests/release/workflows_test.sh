#!/usr/bin/env bash
# Static assertions over .github/workflows/{ci,release}.yml and the manifest
# platforms. Needs mikefarah yq v4 and grep. Prints ok/FAIL lines; exits
# non-zero if any check fails.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"
ci=.github/workflows/ci.yml
rel=.github/workflows/release.yml
canary=.github/workflows/harness-canary.yml
fails=0

ok() { printf 'ok   %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1"; fails=$((fails + 1)); }
check() { # check <description> <command...>
  local desc=$1
  shift
  if "$@" >/dev/null 2>&1; then ok "$desc"; else fail "$desc"; fi
}

no_tag_pins() { ! grep -rEn 'uses: [^@]+@v[0-9]' .github/; }
check "no tag-pinned action under .github/" no_tag_pins

sha_pins() {
  local bad=0 line
  while IFS= read -r line; do
    if ! printf '%s\n' "$line" | grep -Eq 'uses: [A-Za-z0-9_.-]+/[A-Za-z0-9_./-]+@[0-9a-f]{40} # v[0-9][0-9.]*$'; then
      printf 'bad pin: %s\n' "$line" >&2
      bad=1
    fi
  done < <(grep -hE '^[[:space:]-]*uses:' .github/workflows/*.yml)
  [ "$bad" -eq 0 ]
}
check "every uses: is owner/repo@<40-hex> # vX.Y.Z" sha_pins

check "release.yml top-level permissions.contents == read" \
  test "$(yq '.permissions.contents' "$rel")" = read

write_jobs() { yq '.jobs | to_entries | .[] | select(.value.permissions.contents == "write") | .key' "$rel"; }
check "release.yml: exactly one job has contents: write" test "$(write_jobs | wc -l | tr -d ' ')" = 1
publish=$(write_jobs | head -n 1)
check "release.yml: publish job (${publish:-none}) if is push && tag" bash -c '
  cond=$(yq ".jobs.\"$1\".if" "$2")
  case "$cond" in *"github.event_name == '"'"'push'"'"'"*) ;; *) exit 1 ;; esac
  case "$cond" in *"github.ref_type == '"'"'tag'"'"'"*) ;; *) exit 1 ;; esac
' _ "$publish" "$rel"

others_clean() {
  local job
  for job in $(yq '.jobs | keys | .[]' "$rel"); do
    [ "$job" = "$publish" ] && continue
    if yq ".jobs.\"$job\"" "$rel" | grep -q 'gh release'; then
      printf 'job %s calls gh release\n' "$job" >&2
      return 1
    fi
  done
}
check "release.yml: no non-publish job calls gh release" others_clean

check "release.yml: on.push.tags == [v*]" test "$(yq -o=json -I=0 '.on.push.tags' "$rel")" = '["v*"]'
check "release.yml: on.workflow_dispatch exists" test "$(yq '.on | has("workflow_dispatch")' "$rel")" = true

check "ci.yml: build-targets job exists" test "$(yq '.jobs | has("build-targets")' "$ci")" = true
check "ci.yml: build-targets matrix builds the four targets" test \
  "$(yq -o=json -I=0 '[.jobs.build-targets.strategy.matrix.include[].targets | split(" ") | .[]] | sort' "$ci")" = \
  '["aarch64-apple-darwin","aarch64-unknown-linux-musl","x86_64-apple-darwin","x86_64-unknown-linux-musl"]'
check "ci.yml: build-targets builds both darwin targets on one macOS runner" test \
  "$(yq -o=json -I=0 '[.jobs.build-targets.strategy.matrix.include[] | select(.runner | test("^macos")) | .targets]' "$ci")" = \
  '["aarch64-apple-darwin x86_64-apple-darwin"]'
check "ci.yml: build-targets toolchain and build steps read matrix.targets" test \
  "$(yq -o=json -I=0 '[.jobs.build-targets.steps[] | select(.name == "Rust toolchain" or .name == "Locked release build") | .env.TARGETS]' "$ci")" = \
  '["${{ matrix.targets }}","${{ matrix.targets }}"]'
check "ci.yml: build-targets builds --locked --release" bash -c \
  'yq ".jobs.build-targets" "$1" | grep -q -- "cargo build --locked --release"' _ "$ci"
check "ci.yml: clippy job exists" test "$(yq '.jobs | has("clippy")' "$ci")" = true
check "ci.yml: clippy --all-features -D warnings" bash -c \
  'yq ".jobs.clippy.steps[].run" "$1" | grep -qE "^cargo clippy --locked --all-targets --all-features -- -D warnings$"' _ "$ci"
check "ci.yml: clippy default features -D warnings" bash -c \
  'yq ".jobs.clippy.steps[].run" "$1" | grep -qE "^cargo clippy --locked --all-targets -- -D warnings$"' _ "$ci"
check "ci.yml: package-lifecycle cargo test enables test-support" bash -c \
  'yq ".jobs.package-lifecycle.steps[].run" "$1" | grep -qE -- "cargo test --locked (--features test-support|--all-features) --test package"' _ "$ci"
check "ci.yml: deterministic job ends with an always() leak check" bash -c \
  'test "$(yq ".jobs.deterministic.steps[] | select(.run == \"scripts/check-no-leaked-processes*\") | .if" "$1")" = "$2"' _ "$ci" "always() && needs.changes.outputs.heavy == 'true'"
check "ci.yml: tests job runs on Linux in six shards" bash -c \
  'test "$(yq ".jobs.tests.runs-on" "$1")" = ubuntu-24.04 && test "$(yq -o=json -I=0 ".jobs.tests.strategy.matrix.shard" "$1")" = "[1,2,3,4,5,6]"' _ "$ci"
check "ci.yml: tests job partitions nextest by slice of the matrix size" bash -c \
  'yq ".jobs.tests.steps[].run" "$1" | grep -qF -- "--partition \"slice:\$SHARD/\$SHARDS\"" && yq ".jobs.tests.steps[].env.SHARDS" "$1" | grep -qxF "\${{ strategy.job-total }}"' _ "$ci"
check "ci.yml: tests job ends with an always() leak check" bash -c \
  'test "$(yq ".jobs.tests.steps[] | select(.run == \"scripts/check-no-leaked-processes*\") | .if" "$1")" = "$2"' _ "$ci" "always() && needs.changes.outputs.heavy == 'true'"
check "ci.yml: clippy runs on macOS and Linux" bash -c \
  'test "$(yq -o=json -I=0 ".jobs.clippy.strategy.matrix.runner" "$1")" = "[\"macos-15\",\"ubuntu-24.04\"]"' _ "$ci"
check "ci.yml: actionlint job exists" test "$(yq '.jobs | has("actionlint")' "$ci")" = true
check "ci.yml: linux-e2e runs on Linux x86_64 and aarch64" bash -c \
  'test "$(yq -o=json -I=0 "[.jobs.linux-e2e.strategy.matrix.include[].runner] | sort" "$1")" = "[\"ubuntu-24.04\",\"ubuntu-24.04-arm\"]"' _ "$ci"
check "ci.yml: linux-e2e runs the wake-e2e self-test and the Herdr release smoke" bash -c \
  'yq ".jobs.linux-e2e.steps[].run" "$1" | grep -qxF "python3 scripts/wake-e2e.py --self-test" \
   && yq ".jobs.linux-e2e.steps[].run" "$1" | grep -qF "scripts/herdr-release-smoke --cache"' _ "$ci"
check "ci.yml: linux-e2e never runs the live wake matrix or --check-upstream" bash -c \
  'test "$(yq -o=json -I=0 "[.jobs.linux-e2e.steps[].run | select(. != null) | select(test(\"wake-e2e\"))]" "$1")" \
     = "[\"python3 scripts/wake-e2e.py --self-test\"]" \
   && ! yq ".jobs.linux-e2e.steps[].run" "$1" | grep -qF -- "--check-upstream"' _ "$ci"
check "ci.yml: linux-e2e references no secrets or tokens" bash -c \
  '! yq ".jobs.linux-e2e" "$1" | grep -qE "secrets\b|GITHUB_TOKEN|github\.token"' _ "$ci"
check "ci.yml: linux-e2e ends with an always() leak check" bash -c \
  'test "$(yq ".jobs.linux-e2e.steps[] | select(.run == \"scripts/check-no-leaked-processes*\") | .if" "$1")" = "$2" \
   && test "$(yq ".jobs.linux-e2e.steps[-1].run" "$1")" = "scripts/check-no-leaked-processes --run-id \"\$HT_LEAK_RUN_ID\" --grace 15"' _ "$ci" "always() && needs.changes.outputs.heavy == 'true'"
check "ci.yml: linux-e2e runs on every push and pull request (always evaluates detector)" bash -c \
  'test "$(yq ".jobs.linux-e2e.if" "$1")" = "always()" \
   && test "$(yq "(.on | has(\"push\")) and (.on | has(\"pull_request\")) and .on.push == null and .on.pull_request == null" "$1")" = true' _ "$ci"

rust_caches() {
  local job
  for job in tests clippy build-targets release-proof; do
    test "$(yq ".jobs.\"$job\".steps[] | select(.uses == \"Swatinem/rust-cache@*\") | .with.cache-bin" "$ci")" = false >/dev/null || return 1
    # Restoring after toolchain installation ensures the actual rustc identity keys the cache.
    local rust cache
    rust=$(yq ".jobs.\"$job\".steps | to_entries | .[] | select(.value.name == \"Rust toolchain\") | .key" "$ci")
    cache=$(yq ".jobs.\"$job\".steps | to_entries | .[] | select(.value.uses == \"Swatinem/rust-cache@*\") | .key" "$ci")
    [ -n "$rust" ] && [ -n "$cache" ] && [ "$rust" -lt "$cache" ] >/dev/null || return 1
  done
  yq '.jobs.release-proof.steps[] | select(.uses == "Swatinem/rust-cache@*") | .with.workspaces' "$ci" | grep -F '. -> target/failpoints-release' >/dev/null || return 1
  yq '.jobs.release-proof.steps[] | select(.uses == "Swatinem/rust-cache@*") | .with.workspaces' "$ci" | grep -F '. -> target/failpoints-control' >/dev/null
}
check "ci.yml: pinned Rust dependency caches include separate proof targets after toolchain setup" rust_caches

proof_once() {
  local proof='manifest::failpoints_are_compiled_out_of_release_builds'
  yq '.jobs.release-proof.steps[].run' "$ci" | grep -F "cargo test --locked --all-features --test package $proof -- --exact --nocapture" >/dev/null || return 1
  yq '.jobs.tests.steps[].run' "$ci" | grep -F "not (binary(package) & test(=$proof))" >/dev/null || return 1
  ! yq '.jobs.tests.steps[].run' "$ci" | grep -F 'check_failpoints_absent.sh --prebuild'
}
check "ci.yml: release proof retains Rust assertions once outside all timed shards" proof_once

docs_filter() {
  local job
  test "$(yq '.jobs.changes.outputs.heavy' "$ci")" = '${{ steps.paths.outputs.heavy }}' || return 1
  for job in tests deterministic linux-e2e clippy build-targets actionlint release-proof; do
    case "$job" in
      actionlint) test "$(yq -o=json -I=0 '.jobs.actionlint.needs' "$ci")" = '["changes","release-proof"]' || return 1 ;;
      *) test "$(yq ".jobs.\"$job\".needs" "$ci")" = changes || return 1 ;;
    esac
    test "$(yq ".jobs.\"$job\".if" "$ci")" = "always()" || return 1
    yq ".jobs.\"$job\".steps[].if" "$ci" | while IFS= read -r condition; do
      case "$condition" in "always()"|*"needs.changes.outputs.heavy == 'true'"*) ;; *) return 1 ;; esac
    done || return 1
  done
}
check "ci.yml: docs-only filter preserves every stable job and gates its steps" docs_filter
detector_failure_blocks() {
  local job script outcome heavy
  for job in tests deterministic linux-e2e clippy build-targets actionlint release-proof; do
    script=$(yq ".jobs.\"$job\".steps[0].run" "$ci")
    for outcome in failure skipped cancelled; do
      if DETECTOR_RESULT="$outcome" HEAVY=true bash -c "$script"; then return 1; fi
    done
    for heavy in '' invalid; do
      if DETECTOR_RESULT=success HEAVY="$heavy" bash -c "$script"; then return 1; fi
    done
    DETECTOR_RESULT=success HEAVY=false bash -c "$script" || return 1
    DETECTOR_RESULT=success HEAVY=true bash -c "$script" || return 1
  done
  test "$(yq '.jobs.actionlint.steps[-1].env.PROOF_RESULT' "$ci")" = '${{ needs.release-proof.result }}' || return 1
  script=$(yq '.jobs.actionlint.steps[-1].run' "$ci")
  for outcome in failure skipped cancelled; do
    if PROOF_RESULT="$outcome" bash -c "$script"; then return 1; fi
  done
  PROOF_RESULT=success bash -c "$script"
}
check "ci.yml: detector failure/missing output and proof failure fail stable checks" detector_failure_blocks
check "ci.yml: changed-path detector event semantics" python3 -m unittest discover -s scripts/tests -p test_ci_changed_paths.py

full=.github/workflows/full-suite.yml
check "full-suite.yml: manual dispatch only, contents read" bash -c \
  'test "$(yq -o=json -I=0 ".on | keys" "$1")" = "[\"workflow_dispatch\"]" && test "$(yq ".permissions.contents" "$1")" = read' _ "$full"
# The timed step is pinned whole: its script and its env, nothing else.
expected_timed_run=$(cat <<'TIMED'
set -euo pipefail
echo "runner: $(nproc) CPUs"
start=$(date +%s)
rc=0
cargo nextest run --locked --all-targets --all-features 2>&1 | tee full-suite.log || rc=$?
elapsed=$(( $(date +%s) - start ))
echo "Full suite wall time: ${elapsed}s (budget: under 300s), exit ${rc}"
echo "FULL_SUITE_WALL_SECONDS=$elapsed" >> "$GITHUB_ENV"
if [ "$elapsed" -ge 300 ]; then
  echo "::error::The full suite exceeded the five-minute budget (${elapsed}s on $(nproc) CPUs)"
  exit 1
fi
exit "$rc"
TIMED
)
timed_step() { yq ".jobs.full-suite.steps[] | select(.name == \"Full suite*\") | $1" "$full"; }
check "full-suite.yml: the exact unsharded command, timed under 300 s (whole step pinned)" bash -c \
  'test "$1" = "$2"' _ "$(timed_step .run)" "$expected_timed_run"
check "full-suite.yml: timed step env is only CARGO_TERM_COLOR=never" \
  test "$(timed_step '.env' | yq -o=json -I=0)" = '{"CARGO_TERM_COLOR":"never"}'
check "full-suite.yml: no step script sets NEXTEST_* or HT_SKIP_HERDR_TESTS" bash -c \
  '! yq ".jobs.full-suite.steps[].run" "$1" | grep -qE "NEXTEST_|HT_SKIP_HERDR_TESTS="' _ "$full"
check "full-suite.yml: neither the job nor the timed step is conditional" bash -c \
  'test "$(yq ".jobs.full-suite | has(\"if\")" "$1")" = false &&
   test "$(yq ".jobs.full-suite.steps[] | select(.name == \"Full suite*\") | has(\"if\")" "$1")" = false' _ "$full"
check "full-suite.yml: prewarm builds precede the timed step" bash -c '
  steps=$(yq ".jobs.full-suite.steps[].name" "$1")
  warm=$(printf "%s\n" "$steps" | grep -n "^Prewarm" | cut -d: -f1)
  timed=$(printf "%s\n" "$steps" | grep -n "^Full suite" | cut -d: -f1)
  [ -n "$warm" ] && [ -n "$timed" ] && [ "$warm" -lt "$timed" ]' _ "$full"
check "full-suite.yml: no env or step setting weakens the timed run" bash -c '
  ! yq "[.env, .jobs.full-suite.env, .jobs.full-suite.steps[].env] | .[] | select(. != null) | keys | .[]" "$1" | grep -qE "^(NEXTEST_|HT_SKIP_HERDR_TESTS$)" &&
  test "$(yq ".jobs.full-suite.steps[] | select(.name == \"Full suite*\") | has(\"continue-on-error\")" "$1")" = false &&
  test "$(yq ".jobs.full-suite | has(\"continue-on-error\")" "$1")" = false' _ "$full"
check "full-suite.yml: prewarm builds the timed command (--no-run)" bash -c \
  'yq ".jobs.full-suite.steps[] | select(.name == \"Prewarm*\") | .run" "$1" | grep -qxF "cargo nextest run --locked --all-targets --all-features --no-run"' _ "$full"
# The report is pinned whole: it states the configured version and each step's actual outcome.
expected_report_run=$(cat <<'REPORT'
set -euo pipefail
if [ -n "${FULL_SUITE_WALL_SECONDS:-}" ]; then wall="${FULL_SUITE_WALL_SECONDS}s"; else wall="not measured"; fi
{
  echo "### Full suite gate"
  echo "Commit: $GITHUB_SHA; runner CPUs: $(nproc); wall: $wall (budget under 300s); timed run: ${TIMED_OUTCOME:-not run}"
  echo "Herdr $HERDR_VERSION (configured): provisioning ${HERDR_OUTCOME:-not run}; provisioned-not-skipped check ${HERDR_CHECK_OUTCOME:-not run}; HT_SKIP_HERDR_TESTS: ${HT_SKIP_HERDR_TESTS:-unset}."
  echo '```'
  grep -E '^ +Summary|^ +(FAIL|SIGKILL|TIMEOUT|LEAK)' full-suite.log 2>/dev/null | sort -u || true
  echo '```'
} >> "$GITHUB_STEP_SUMMARY"
REPORT
)
report_step() { yq ".jobs.full-suite.steps[] | select(.name == \"Report*\") | $1" "$full"; }
check "full-suite.yml: report script pinned whole" bash -c 'test "$1" = "$2"' _ "$(report_step .run)" "$expected_report_run"
check "full-suite.yml: report env is the three step outcomes" \
  test "$(report_step '.env' | yq -o=json -I=0)" = '{"HERDR_OUTCOME":"${{ steps.herdr.outcome }}","HERDR_CHECK_OUTCOME":"${{ steps.herdr_check.outcome }}","TIMED_OUTCOME":"${{ steps.timed.outcome }}"}'
step_id() { yq ".jobs.full-suite.steps[] | select(.name == \"$1*\") | .id" "$full"; }
check "full-suite.yml: outcome ids sit on their own steps" bash -c \
  'test "$1" = herdr && test "$2" = herdr_check && test "$3" = timed && test "$4" = null' _ \
  "$(step_id 'Provision pinned Herdr')" "$(step_id 'Isolated Herdr tests')" "$(step_id 'Full suite')" "$(step_id 'Prewarm')"
check "full-suite.yml: run-scoped always() leak check" bash -c \
  'test "$(yq ".jobs.full-suite.steps[] | select(.run == \"scripts/check-no-leaked-processes*\") | .if" "$1")" = "always()" &&
   yq ".jobs.full-suite.env.HT_LEAK_RUN_ID" "$1" | grep -qF "github.run_id"' _ "$full"

check "herdr-plugin.toml platforms = [\"macos\", \"linux\"]" \
  grep -qxF 'platforms = ["macos", "linux"]' herdr-plugin.toml

canary_writers() { yq '.jobs | to_entries | .[] | select(.value.permissions.contents == "write") | .key' "$canary"; }
check "harness-canary.yml: exactly one job has contents: write, publish-manifest" \
  test "$(canary_writers)" = publish-manifest
check "harness-canary.yml: publish-manifest dispatch requires the default branch" bash -c '
  cond=$(yq ".jobs.publish-manifest.if" "$1")
  case "$cond" in *"github.ref == format('"'"'refs/heads/{0}'"'"', github.event.repository.default_branch)"*) ;; *) exit 1 ;; esac
  case "$cond" in *"github.event_name == '"'"'workflow_dispatch'"'"'"*) ;; *) exit 1 ;; esac
' _ "$canary"
ref_guard_first() {
  local guard push
  guard=$(yq '.jobs.publish-manifest.steps | to_entries | .[] | select((.value.run // "") | test("(?s)GITHUB_REF.*\\.default_branch|\\.default_branch.*GITHUB_REF")) | .key' "$canary" | head -n 1)
  push=$(yq '.jobs.publish-manifest.steps | to_entries | .[] | select((.value.run // "") | test("git push")) | .key' "$canary" | head -n 1)
  [ -n "$guard" ] && [ -n "$push" ] && [ "$guard" -lt "$push" ]
}
check "harness-canary.yml: publish-manifest checks GITHUB_REF before any push" ref_guard_first

if [ "$fails" -ne 0 ]; then
  printf '%d check(s) failed\n' "$fails" >&2
  exit 1
fi
echo "all workflow checks passed"
