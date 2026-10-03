#!/bin/sh
# ht-4is.11.1: prove crash-boundary failpoints are compiled out of ordinary
# (release, no-feature) builds, with a positive control on a test-support build.
#
# Usage: tests/service/check_failpoints_absent.sh [target-dir]
# Kills: a failpoint hook (or its registry) left reachable in release.
set -eu
root=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd -P)
cd "$root"
target=${1:-${CARGO_TARGET_DIR:-$root/target}}
names=$(find src -name '*.rs' -exec cat {} + | tr '\n' ' ' \
  | grep -oE 'failpoint!\( *"[a-z_.]+"' | sed -E 's/.*"([a-z_.]+)"/\1/' | sort -u)
count=$(printf '%s\n' "$names" | grep -c . || true)
[ "$count" -gt 0 ] || { echo "FAIL: no failpoint hooks found in src"; exit 1; }
echo "failpoint hooks in source: $count"
printf '  %s\n' $names

CARGO_TARGET_DIR="$target/failpoints-release" cargo build --locked --release --quiet
release="$target/failpoints-release/release/herdr-threads"
CARGO_TARGET_DIR="$target/failpoints-control" cargo build --locked --release --features test-support --quiet
control="$target/failpoints-control/release/herdr-threads"

status=0
for name in $names test_support::failpoints "test failpoint"; do
  if LC_ALL=C grep -aqF "$name" "$release"; then
    echo "FAIL: release binary contains '$name'"
    status=1
  fi
done
present=0
for name in $names; do
  if LC_ALL=C grep -aqF "$name" "$control"; then present=$((present + 1)); fi
done
echo "positive control (test-support build) contains $present/$count hook names"
[ "$present" -eq "$count" ] || { echo "FAIL: positive control incomplete"; status=1; }
[ "$status" -eq 0 ] && echo "PASS: release binary $(shasum -a 256 "$release" | cut -c1-16) contains 0/$count failpoint names"
exit "$status"
