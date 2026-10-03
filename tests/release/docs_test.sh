#!/usr/bin/env bash
# Static assertions over the v0.1.0 release documents: docs/release.md,
# CHANGELOG.md, README.md and docs/install.md. Uses grep only (no ripgrep
# dependency). Prints ok/FAIL lines; exits non-zero if any check fails.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"
fails=0

ok() { printf 'ok   %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1"; fails=$((fails + 1)); }
check() { # check <description> <command...>
  local desc=$1
  shift
  if "$@" >/dev/null 2>&1; then ok "$desc"; else fail "$desc"; fi
}

# The user-facing documents. The run and history folders quote the false claim
# and the old wording on purpose, so they are not scanned.
docs=(README.md CHANGELOG.md docs/*.md docs/compatibility/*.md docs/validation/*.md)

no_stale_claim() { ! grep -n 'unchanged since d635aca' "${docs[@]}"; }
check 'no "unchanged since d635aca" claim in README/docs' no_stale_claim

# Installer replacement is crash-safe (two renames), never "atomic". The ACK
# batch is genuinely atomic, so lines about ACKs and batches are exempt.
no_atomic_install() {
  ! grep -in 'atomic' README.md docs/install.md docs/release.md | grep -viE 'ack|batch'
}
check 'no "atomic" install wording in README/install/release docs' no_atomic_install

# CHANGELOG.md: a v0.1.0 section with the five items.
section() { # section FILE HEADING-REGEX: lines of the section, up to the next "## "
  awk -v re="$2" '$0 ~ re {on=1; next} on && /^## / {exit} on' "$1"
}
changelog_has() { section CHANGELOG.md '^## v0\.1\.0' | grep -Eq "$1"; }
check 'CHANGELOG.md has a "## v0.1.0" heading' grep -q '^## v0\.1\.0' CHANGELOG.md
check 'CHANGELOG: removed STATUS constants' changelog_has 'EXIT_STATUS_HELP'
check 'CHANGELOG: inv- prefix' changelog_has 'inv-'
check 'CHANGELOG: manifest v2 downgrade refusal' changelog_has 'manifest version 2.*(older|downgrad)|(older|downgrad).*manifest version 2'
check 'CHANGELOG: optimistic admission with the shipped wording' changelog_has 'optimistic — newer than verified'
check 'CHANGELOG: platforms macos and linux' changelog_has 'platforms = \["macos", "linux"\]'

# docs/release.md: the tag-only publish flow.
rel=docs/release.md
rel_has() { grep -Eq "$1" "$rel"; }
check 'release.md: publish only from a v* tag push' rel_has 'only .*tag push|tag-only'
check 'release.md: idempotent draft then upload --clobber then checksum verify then publish' rel_has 'draft.*--clobber.*(checksum|sha256sum).*publish'
check 'release.md: build-targets job' rel_has 'build-targets'

# docs/release.md: every root spec Follow-on item, under one heading.
follow=$(section "$rel" '^## Post-merge follow-on checklist')
follow_has() { printf '%s\n' "$follow" | grep -Eq "$1"; }
check 'release.md: follow-on section exists' test -n "$follow"
for n in 1 2 3 4 5 6; do
  check "release.md: follow-on item $n" follow_has "^$n\\. "
done
check 'follow-on 1: first CI run, build-targets all four targets' follow_has '^1\..*build-targets'
check 'follow-on 2: API key secrets' follow_has 'ANTHROPIC_API_KEY.*OPENAI_API_KEY|OPENAI_API_KEY.*ANTHROPIC_API_KEY'
check 'follow-on 2: 60-day scheduled workflow disable' follow_has '60 days'
check 'follow-on 2: gh workflow view harness-canary.yml' follow_has 'gh workflow view harness-canary\.yml'
check 'follow-on 2: gh workflow enable harness-canary.yml' follow_has 'gh workflow enable harness-canary\.yml'
check 'follow-on 2: workflow_dispatch of harness-canary.yml' follow_has 'workflow_dispatch.*harness-canary\.yml|harness-canary\.yml.*workflow_dispatch'
check 'follow-on 3: push tag v0.1.0' follow_has '^3\..*v0\.1\.0'
check 'follow-on 4: clean-machine rehearsal on macOS and Linux' follow_has '^4\..*rehearsal'
check 'follow-on 5: Linux link refused, flip platforms' follow_has '^5\..*platforms'
check 'follow-on 6: tests with no thread pin on both OSes' follow_has '^6\..*(--test-threads|thread pin)'

# One workflow_dispatch run per configuration the canary smoke recorded as
# NOT_EXERCISED (docs/history/*/canary-smoke.md, written by
# ht-p03.14.7). The table's first cell names the configuration; release.md must
# carry that text. The three configurations the smoke task defines are asserted
# unconditionally, so a missing smoke file does not hide a missing row.
for f in docs/history/*/canary-smoke.md; do
  [ -f "$f" ] || continue
  while IFS= read -r cfg; do
    [ -n "$cfg" ] || continue
    check "follow-on dispatch covers NOT_EXERCISED configuration: $cfg" follow_has "$cfg"
  done < <(grep 'NOT_EXERCISED' "$f" | awk -F'|' 'NF > 2 {gsub(/^ +| +$/, "", $2); gsub(/`/, "", $2); print $2}')
done
check 'follow-on dispatch: ubuntu-24.04 x86_64 install layout, tier 0' follow_has 'linux/amd64|ubuntu-24\.04 \(Linux x86_64\)'
check 'follow-on dispatch: since-verified --bisect on the real registry' follow_has 'since-verified --bisect'
check 'follow-on dispatch: live gh issue filing step' follow_has 'file_issues\.py|issue filing'

# README: the Linux link is unverified until the follow-on rehearsal.
readme_linux() { grep -iE 'linux' README.md | grep -iE 'unverified' | grep -qiE 'rehearsal'; }
check 'README: Linux link unverified until the follow-on rehearsal' readme_linux

if [ "$fails" -ne 0 ]; then
  printf 'DOCS_TEST_FAIL %d check(s) failed\n' "$fails"
  exit 1
fi
printf 'DOCS_TEST_PASS\n'
