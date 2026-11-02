#!/bin/bash
cd /var/home/alepar/AleCode/herdr-threads/.worktrees/external-seat/.worktrees/super-auto-environments-named-seats-herdr-app--task-ht-kqrt.229
export TMPDIR=/tmp/sp-503081fe/ht-kqrt.229 CARGO_INCREMENTAL=0
pids=()
trap 'kill "${pids[@]}" 2>/dev/null' EXIT
for i in 1 2 3 4; do (taskset -c 0,1 nice -n 19 sh -c 'while :; do :; done') & pids+=($!); done
pass=0; fail=0
for i in $(seq 20); do
  if taskset -c 0,1 nice -n 19 cargo nextest run --locked --all-features --test service --retries 0 -E 'test(store_only_daemon_serves_without_host_workers)' > $TMPDIR/starve-$i.log 2>&1; then pass=$((pass+1)); else fail=$((fail+1)); fi
done
echo "pass=$pass fail=$fail" > $TMPDIR/starve.result
