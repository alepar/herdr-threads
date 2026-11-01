#!/bin/bash
cd /var/home/alepar/AleCode/herdr-threads/.worktrees/external-seat/.worktrees/super-auto-environments-named-seats-herdr-app--task-ht-kqrt.62
export TMPDIR=/tmp/sp-503081fe/ht-kqrt.62 CARGO_INCREMENTAL=0
H=/var/home/alepar/.cache/herdr-threads-heavy/heavy
run(){ n=$1; shift; echo "### $n: $*"; "$@" > evidence/ht-kqrt.62/final/$n.log 2>&1; echo "### $n exit=$?"; }
run wire-compat scripts/check-wire-compat
run harness-neutral scripts/check-harness-neutral
run host-neutral-scan $H cargo test --locked --all-features --test combined host_neutral_scan
run default-features $H nice scripts/check-default-features
run wake-selftest python3 scripts/wake-e2e.py --self-test
run release-smoke $H nice scripts/herdr-release-smoke --versions 0.9.5
echo ALLDONE
