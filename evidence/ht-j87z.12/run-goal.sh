#!/bin/bash
cd "$1"
run(){ echo "### $*"; nice cargo test --locked --all-features "$@" 2>&1 | grep -E '^test result|^error|FAILED|panicked|warning: unused|Running'; echo "rc=${PIPESTATUS[0]}"; }
run --test opaque_relay
run --test combined peer_wire
run --test combined forward_compat
run --test combined journal_port_contract
echo ALLDONE
