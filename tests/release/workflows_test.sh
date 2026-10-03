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
check "ci.yml: build-targets matrix has the four targets" test \
  "$(yq -o=json -I=0 '[.jobs.build-targets.strategy.matrix.include[].target] | sort' "$ci")" = \
  '["aarch64-apple-darwin","aarch64-unknown-linux-musl","x86_64-apple-darwin","x86_64-unknown-linux-musl"]'
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
  'test "$(yq ".jobs.deterministic.steps[] | select(.run == \"scripts/check-no-leaked-processes*\") | .if" "$1")" = "always()"' _ "$ci"
check "ci.yml: actionlint job exists" test "$(yq '.jobs | has("actionlint")' "$ci")" = true

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
