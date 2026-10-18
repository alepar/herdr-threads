#!/bin/bash
cd "$1"; D=evidence/ht-j87z.12
s=$(date +%s)
nice cargo nextest run --locked --all-targets --all-features > $D/suite.log 2>&1; echo rc=$? >> $D/suite.log
echo wall=$(( $(date +%s)-s ))s >> $D/suite.log
scripts/check-no-leaked-processes --run-id "$HT_LEAK_RUN_ID" > $D/leak.log 2>&1; echo rc=$? >> $D/leak.log
touch $D/suite.done
