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
# all dependencies twice and starved timing-sensitive tests.
#
# The two configurations build concurrently: each is mostly one crate's
# serial frontend and its binary, so one after the other they left the
# proof the suite's last test. Cargo locks a whole target directory, so the
# control builds in a copy of the shared release tree (clonefile on macOS,
# else a reflink or plain copy, modification times kept so Cargo's freshness
# checks see the same tree): dependencies built in the shared tree, and a
# control CI already built there, are reused. A cold tree builds its
# dependencies in both directories.
mkdir -p "$target"
# A run killed past nextest's grace period leaves its proof directory (and
# its copy of the release tree). Prune only old ones: a proof running beside
# this one (a focused run next to a full suite) keeps its copy.
find "$target" -maxdepth 1 -name 'failpoints-proof.*' -mmin +30 -exec rm -rf {} + 2>/dev/null || true
proof=$(mktemp -d "$target/failpoints-proof.XXXXXX")
pids=
# Cargo does not pass a TERM on to its rustc children: stop them first.
cleanup() {
  for pid in $pids; do
    pkill -TERM -P "$pid" 2>/dev/null || true
    kill "$pid" 2>/dev/null || true
  done
  rm -rf "$proof"
}
trap cleanup EXIT
trap 'cleanup; exit 1' HUP INT TERM
clone="$proof/control-target"
mkdir -p "$clone"
if [ -d "$target/release" ]; then
  cp -c -R "$target/release" "$clone/release" 2>/dev/null \
    || { rm -rf "$clone/release"; cp -pR --reflink=auto "$target/release" "$clone/release" 2>/dev/null; } \
    || { rm -rf "$clone/release"; cp -pR "$target/release" "$clone/release"; }
fi
CARGO_TARGET_DIR="$target" nice -n 19 cargo build --locked --release --quiet &
release_build=$!
CARGO_TARGET_DIR="$clone" nice -n 19 cargo build --locked --release --features test-support --quiet &
control_build=$!
pids="$release_build $control_build"
built=0
wait "$release_build" || built=$?
wait "$control_build" || built=$?
pids=
[ "$built" -eq 0 ] || { echo "FAIL: release proof build exited $built"; exit 1; }
cp "$target/release/herdr-threads" "$proof/release"
cp "$clone/release/herdr-threads" "$proof/control"
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
