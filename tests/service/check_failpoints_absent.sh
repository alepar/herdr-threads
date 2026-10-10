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

# Reuse one Cargo cache for the two feature configurations. CI already
# builds both releases before the suite; separate cold trees rebuilt
# all dependencies twice and starved timing-sensitive tests. Preserve each
# binary before the next build overwrites Cargo's shared output path.
mkdir -p "$target"
proof=$(mktemp -d "$target/failpoints-proof.XXXXXX")
trap 'rm -rf "$proof"' EXIT HUP INT TERM
CARGO_TARGET_DIR="$target" nice -n 19 cargo build --locked --release --quiet
cp "$target/release/herdr-threads" "$proof/release"
CARGO_TARGET_DIR="$target" nice -n 19 cargo build --locked --release --features test-support --quiet
cp "$target/release/herdr-threads" "$proof/control"
release="$proof/release"
control="$proof/control"

status=0
# The thread-local launch-composition drift seam is test-only as well.
drift="simulated current composer drift"
for name in $names test_support::failpoints "test failpoint" "$drift"; do
  if LC_ALL=C grep -aqF "$name" "$release"; then
    echo "FAIL: release binary contains '$name'"
    status=1
  fi
done
present=0
for name in $names; do
  if LC_ALL=C grep -aqF "$name" "$control"; then present=$((present + 1)); fi
done
LC_ALL=C grep -aqF "$drift" "$control" || { echo "FAIL: positive control lacks drift seam"; status=1; }
echo "positive control (test-support build) contains $present/$count hook names"
[ "$present" -eq "$count" ] || { echo "FAIL: positive control incomplete"; status=1; }
[ "$status" -eq 0 ] && echo "PASS: release binary $(shasum -a 256 "$release" | cut -c1-16) contains 0/$count failpoint names"
exit "$status"
