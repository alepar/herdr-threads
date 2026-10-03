#!/bin/sh
# witness.sh — records one hook invocation, then runs the real herdr-threads (nested spec §D3 t0.hook-fires).
#
# harness-canary.sh installs this script as P/ht/herdr-threads after setup, with the real binary saved beside
# it as P/ht/herdr-threads.real, so the hook command setup wrote (which names P/ht/herdr-threads) reaches it.
# Per invocation it writes P/capture/tier0/<n>.stdin (raw stdin) and <n>.argv (one argument per line, the
# arguments after the program name), the .argv last and each by atomic rename so a reader that sees an .argv
# sees its .stdin; it then execs the real binary with the same arguments and the same stdin. <n> is
# <epoch seconds>-<pid>, which sorts by capture time and cannot collide between concurrent hooks.
# harness-canary.sh puts the real binary back before any check that must not be recorded (§D5, coverage r2).
self=$0
case $self in /*) ;; *) self=$PWD/$self ;; esac
dir=$(dirname "$self")
real=$self.real
cap=$dir/../capture/tier0
mkdir -p "$cap"
n=$(date +%s)-$$
cat > "$cap/.$n.stdin.tmp"
{ for a in "$@"; do printf '%s\n' "$a"; done; } > "$cap/.$n.argv.tmp"
mv "$cap/.$n.stdin.tmp" "$cap/$n.stdin"
mv "$cap/.$n.argv.tmp" "$cap/$n.argv"
exec "$real" "$@" < "$cap/$n.stdin"
