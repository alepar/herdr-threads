#!/bin/sh
# ht-4is.11.1: prove crash-boundary failpoints are compiled out of ordinary
# (release, no-feature) builds, with a positive control on a test-support build.
#
# Usage: tests/service/check_failpoints_absent.sh [--prebuild] [target-dir]
#   --prebuild: only build the two release proof fixtures (CI runs this before
#   the timed suite, so the suite's builds are no-ops).
# Kills: a failpoint hook (or its registry) left reachable in release.
set -eu
prebuild=0
if [ "${1:-}" = "--prebuild" ]; then prebuild=1; shift; fi
root=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd -P)
cd "$root"
target=${1:-${CARGO_TARGET_DIR:-$root/target}}
names=$(find src -name '*.rs' -exec cat {} + | tr '\n' ' ' \
  | grep -oE 'failpoint!\( *"[a-z_.]+"' | sed -E 's/.*"([a-z_.]+)"/\1/' | sort -u)
count=$(printf '%s\n' "$names" | grep -c . || true)
[ "$count" -gt 0 ] || { echo "FAIL: no failpoint hooks found in src"; exit 1; }
echo "failpoint hooks in source: $count"
printf '  %s\n' $names

# The two release configurations (ordinary, then test-support as the
# positive control) build concurrently, each in its own persistent Cargo
# target directory under the shared target (Cargo locks a whole directory),
# incrementally: after the first run an edit rebuilds only what changed
# (a one-line edit: ~10 s instead of ~5 min of release codegen). The
# directories are seeded once from "$target/release" (clonefile on macOS,
# else a reflink or a copy with modification times kept, so Cargo's
# freshness checks see the same tree) and never deleted, so dependencies and
# incremental state are reused across runs. CI builds them before the timed
# suite with --prebuild.
mkdir -p "$target"
# A run killed past nextest's grace period leaves its proof directory (or a
# seeding lock). Prune only old ones: a proof running beside this one keeps
# its copies.
prune() {
  find "$target" -maxdepth 1 \( -name 'failpoints-proof.*' -o -name 'failpoints-*.seeding' \) -mmin +30 -exec rm -rf {} + 2>/dev/null || true
}
prune
ordinary_dir="$target/failpoints-release"
control_dir="$target/failpoints-control"
pids=
proof=
held=
# Cargo does not pass a TERM on to its rustc children: stop them first.
stop_builds() {
  for pid in $pids; do
    pkill -TERM -P "$pid" 2>/dev/null || true
    kill "$pid" 2>/dev/null || true
  done
}
cleanup() {
  stop_builds
  [ -z "$held" ] || rm -rf "$held"
  [ -z "$proof" ] || rm -rf "$proof"
}
trap cleanup EXIT
trap 'cleanup; exit 1' HUP INT TERM
# Seed "$1" once from the release tree "$2". The seeding lock is a directory:
# a concurrent first run waits for the seed instead of copying into the same
# tree; a lock left by a killed run is released by its trap, or pruned once
# stale.
seed() {
  while [ ! -d "$1" ] && [ -d "$2" ]; do
    if mkdir "$1.seeding" 2>/dev/null; then
      held="$1.seeding"
      tmp="$held/tree"
      mkdir -p "$tmp"
      cp -c -R "$2" "$tmp/release" 2>/dev/null \
        || { rm -rf "$tmp/release"; cp -pR --reflink=auto "$2" "$tmp/release" 2>/dev/null; } \
        || { rm -rf "$tmp/release"; cp -pR "$2" "$tmp/release"; }
      mv "$tmp" "$1"
      rm -rf "$held"
      held=
    else
      prune
      sleep 1
    fi
  done
}
build() {
  dir=$1
  shift
  CARGO_INCREMENTAL=1 CARGO_TARGET_DIR="$dir" nice -n 19 cargo build --locked --release --quiet "$@" &
  pids="$pids $!"
}
built=0
wait_builds() {
  for pid in $pids; do wait "$pid" || built=$?; done
  pids=
  [ "$built" -eq 0 ] || { echo "FAIL: release proof build exited $built"; exit 1; }
}
seed "$ordinary_dir" "$target/release"
if [ -d "$target/release" ] || [ -d "$control_dir" ]; then
  seed "$control_dir" "$target/release"
else
  # A cold tree (a fresh CI runner): build the ordinary release once, then
  # seed the control from it, so dependencies build once.
  build "$ordinary_dir"
  wait_builds
  seed "$control_dir" "$ordinary_dir/release"
fi
build "$ordinary_dir"
build "$control_dir" --features test-support
wait_builds
[ "$prebuild" -eq 0 ] || { echo "release proof fixtures built"; exit 0; }
proof=$(mktemp -d "$target/failpoints-proof.XXXXXX")
cp "$ordinary_dir/release/herdr-threads" "$proof/release"
cp "$control_dir/release/herdr-threads" "$proof/control"
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
