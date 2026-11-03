#!/bin/bash
cd /var/home/alepar/AleCode/herdr-threads/.worktrees/external-seat/.worktrees/super-auto-environments-named-seats-herdr-app--task-ht-kqrt.62
export TMPDIR=/tmp/sp-503081fe/ht-kqrt.62 CARGO_INCREMENTAL=0
export HT_LEAK_RUN_ID=$(uuidgen)
echo "RUN_ID=$HT_LEAK_RUN_ID"
H=/var/home/alepar/.cache/herdr-threads-heavy/heavy
s=$(date +%s)
$H nice cargo nextest run --locked --all-targets --all-features
echo "NEXTEST_EXIT=$? secs=$(( $(date +%s)-s ))"
scripts/check-no-leaked-processes --run-id "$HT_LEAK_RUN_ID"
echo "LEAK_EXIT=$?"
