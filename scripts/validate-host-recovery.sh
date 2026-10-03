#!/bin/sh
# Isolated Herdr identity and recovery validation (ht-4is.11.5).
#
# usage: scripts/validate-host-recovery.sh [--bin PATH] [--evidence-out DIR] [--only R02,R04] [--deadline-seconds N] [--run-root DIR]
#
# Starts a PRIVATE Herdr 0.9.1 server (private HOME/XDG/config/API socket under a fresh
# /private/tmp/htr-* run directory) and a test herdr-threads daemon with its own state dir, then
# exercises rename/reorder, stand-in CLI exit, host socket denial and unavailability, test-daemon
# SIGKILL with deadlines, missed events across daemon downtime, registered-seat move, pane close,
# a cooperative idle wake (one overdue-warning prompt into an idle stand-in agent, never a shell),
# host stop/restore with operator repair, and instance namespace separation.
# Never contacts, stops or restarts the shared Herdr server; signals only the private servers it
# started and the test daemons it ensured (argv checked first); launches no model (panes run a
# stand-in script). Exits 0 only when every scenario PASSes; results.json/report.md in the run dir.
# Default --bin: target/release/herdr-threads, built with `cargo build --release --locked`.
set -eu
script_dir=$(CDPATH='' cd "$(dirname "$0")" && pwd -P)
root=$(CDPATH='' cd "$script_dir/.." && pwd -P)
umask 077
case " $* " in
  *" --bin "*) ;;
  *) (cd "$root" && CARGO_TARGET_DIR="$root/target" cargo build --release --locked --quiet)
     set -- --bin "$root/target/release/herdr-threads" "$@" ;;
esac
exec python3 "$root/tests/native/recovery/run_recovery.py" "$@"
