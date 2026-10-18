#!/bin/bash
cd "$1"; D=evidence/ht-j87z.12
nice cargo clippy --locked --all-targets --all-features -- -D warnings > $D/clippy.log 2>&1; echo rc=$? >> $D/clippy.log
scripts/check-harness-neutral > $D/neutral.log 2>&1; echo rc=$? >> $D/neutral.log
nice scripts/check-default-features > $D/default-features.log 2>&1; echo rc=$? >> $D/default-features.log
cargo fmt --check > $D/fmt.log 2>&1; echo rc=$? >> $D/fmt.log
echo ALLDONE > $D/checks.done
