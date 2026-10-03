#!/bin/sh
set -eu

package_root=$(CDPATH='' cd "$(dirname "$0")/.." && pwd -P)
cd "$package_root"
# A release archive (scripts/package-release.sh) ships a prebuilt executable
# and no sources. It carries a PREBUILT marker: keep the shipped binary instead
# of attempting a cargo build that cannot succeed without the sources.
if [ -f PREBUILT ]; then
    if [ -x bin/herdr-threads ]; then
        printf '%s\n' 'herdr-threads: prebuilt release package; using bin/herdr-threads'
        exit 0
    fi
    printf '%s\n' 'herdr-threads: prebuilt release package is missing bin/herdr-threads; reinstall it' >&2
    exit 1
fi
# Pin the target dir: an inherited CARGO_TARGET_DIR would otherwise leave
# target/release missing or stale and install the wrong executable.
CARGO_TARGET_DIR="$package_root/target" cargo build --release --locked
mkdir -p bin
temporary=$(mktemp "$package_root/bin/.herdr-threads.XXXXXX")
trap 'rm -f "$temporary"' EXIT HUP INT TERM
cp target/release/herdr-threads "$temporary"
chmod 755 "$temporary"
mv -f "$temporary" bin/herdr-threads
trap - EXIT HUP INT TERM
