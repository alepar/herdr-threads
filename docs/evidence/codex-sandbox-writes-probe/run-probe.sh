#!/bin/sh
# Codex workspace-write sandbox write probe (no model), ht-4is.8.20.
# Usage: SCRATCH=<fresh dir under /private/tmp> TARGET=<cargo target dir> sh run-probe.sh
# Everything lives under $SCRATCH: HOME, the herdr-threads state dir (the Herdr default layout
# $HOME/.local/state/herdr/plugins/herdr-threads), a fake Herdr host socket, and two scratch
# CODEX_HOMEs. The real ~/.codex, ~/.local and Herdr server are never used.
# /private/tmp is itself writable under workspace-write, so every sandboxed command passes
# exclude_slash_tmp=true and exclude_tmpdir_env_var=true: the state dir is then as unwritable as
# a real ~/.local/state one (B1 proves it), and the cwd ($SCRATCH/ws) is the only workspace root.
set -u
S=$SCRATCH
H=$TARGET/debug/herdr-threads
CODEX=$(command -v codex)
HERE=$(cd "$(dirname "$0")" && pwd)
export HOME=$S/home
ST=$HOME/.local/state/herdr/plugins/herdr-threads
FH=$S/fh.sock
mkdir -p "$ST" "$S/ws" "$S/ch-socket" "$S/ch-setup"; chmod 700 "$ST"
unset HERDR_PLUGIN_STATE_DIR HERDR_SOCKET_PATH HERDR_PANE_ID HERDR_ENV HERDR_BIN_PATH
python3 "$HERE/fakehost.py" "$FH" >/dev/null 2>&1 &
FHPID=$!
sleep 1
base() { "$H" --json --state-dir "$ST" --host-endpoint "$FH" "$@"; }
echo "# Codex $($CODEX --version) workspace-write write probe (no model), $(date -u +%Y-%m-%dT%H:%M:%SZ)"
REAL=$(python3 -c "import os;print(os.path.realpath('$CODEX'))")
echo "# codex binary: $REAL sha256 $(shasum -a 256 "$REAL" | cut -c1-64)"
echo "# herdr-threads: $(git -C "$HERE" rev-parse --short HEAD) (working tree of lane/sandbox-writes)"
base daemon ensure >/dev/null || { echo "daemon ensure failed"; exit 1; }
I=$(ls -d "$ST"/instances/*)
DSOCK=$(python3 -c "import json;print(json.load(open('$I/endpoint.json'))['endpoint'])")
A=$(base seat resolve --pane w1:p1 | python3 -c "import json,sys;print(json.load(sys.stdin)['result']['data'])")
B=$(base seat resolve --pane w1:p2 | python3 -c "import json,sys;print(json.load(sys.stdin)['result']['data'])")
for who in a:$A:w1:p1 b:$B:w1:p2; do
  n=${who%%:*}; rest=${who#*:}; seat=${rest%%:*}; pane=${rest#*:}
  printf '#!/bin/sh\nexec %s --json --state-dir %s --host-endpoint %s --cooperative-seat %s --cooperative-target %s --cooperative-harness codex --cooperative-role top-level "$@"\n' \
    "$H" "$ST" "$FH" "$seat" "$pane" > "$S/ht-$n"; chmod +x "$S/ht-$n"
done
echo "# state=\$SCRATCH/home/.local/state/herdr/plugins/herdr-threads  instance=\$INSTANCE  daemon socket=$DSOCK"
echo "# seats: A=$A (w1:p1)  B=$B (w1:p2); ht-a / ht-b = herdr-threads --json --state-dir ... --cooperative-seat <A|B> ... --cooperative-harness codex --cooperative-role top-level"

# CODEX_HOME "ch-socket": the allowance setup wrote before this change (socket keys only).
cat > "$S/ch-socket/config.toml" <<TOML
[sandbox_workspace_write]
network_access = true

[features.network_proxy]
enabled = true
unix_sockets = { "$DSOCK" = "allow" }
TOML
# CODEX_HOME "ch-setup": written by this branch's `setup codex` against the installed codex.
CODEX_HOME="$S/ch-setup" base setup codex --harness-binary "$REAL" > "$S/setup.json" 2>&1
echo "# setup codex (CODEX_HOME=ch-setup): action=$(python3 -c "import json;d=json.load(open('$S/setup.json'));d=d.get('setup',d);print(d.get('action'), 'writable_roots_present=%s' % d['sandbox'].get('writable_roots_present'))" 2>/dev/null || cat "$S/setup.json")"
echo "# ch-setup/config.toml as written by setup:"
sed 's/^/#   /' "$S/ch-setup/config.toml"
echo

# `rc` after a pipeline is head's; print the command's own status instead.
sbx() { ch=$1; shift; echo "\$ [CODEX_HOME=$ch] $*"; out=$(cd "$S/ws" && CODEX_HOME="$S/$ch" timeout 60 "$CODEX" sandbox \
  -c 'sandbox_mode="workspace-write"' -c sandbox_workspace_write.exclude_slash_tmp=true \
  -c sandbox_workspace_write.exclude_tmpdir_env_var=true -- "$@" 2>&1); rc=$?; printf '%s\n' "$out" | head -c 400; echo "rc=$rc"; echo; }
un() { echo "\$ [unsandboxed] $*"; out=$("$@" 2>&1); rc=$?; printf '%s\n' "$out" | head -c 300; echo "rc=$rc"; echo; }

echo "## P0 unsandboxed preparation: A checks in, creates a thread, invites B, sends B a required-ACK message"
un "$S/ht-a" check-in --lifecycle-event probe-a0
T=$("$S/ht-a" thread create --topic "sandbox write probe" | python3 -c "import json,sys;print(json.load(sys.stdin)['result']['data'])")
echo "thread=$T"
un "$S/ht-a" invite "$T" --seat "$B"
un "$S/ht-a" send "$T" --body "unsandboxed handoff" --require-ack "$B"

echo "## B before: workspace-write, socket allowance only (pre-fix setup)"
sbx ch-socket touch "$I/intents/x"
sbx ch-socket "$S/ht-a" send "$T" --body "sandboxed send, before"
sbx ch-socket "$S/ht-b" check-in --lifecycle-event probe-b0
sbx ch-socket "$S/ht-b" accept "$T"
sbx ch-socket "$S/ht-a" daemon health

echo "## R reset: remove the client journal directories so the sandboxed CLI must create its roots"
rm -rf "$I/intents" "$I/contexts"; ls "$I" | tr '\n' ' '; echo; echo

echo "## A after: workspace-write, config.toml written by setup (socket allowance + writable roots)"
sbx ch-setup "$S/ht-a" check-in --lifecycle-event probe-a1
sbx ch-setup "$S/ht-a" send "$T" --body "sandboxed send, after" --require-ack "$B"
MSG=$(cd "$S/ws" && CODEX_HOME="$S/ch-setup" "$CODEX" sandbox -c 'sandbox_mode="workspace-write"' -c sandbox_workspace_write.exclude_slash_tmp=true -c sandbox_workspace_write.exclude_tmpdir_env_var=true -- "$S/ht-a" send "$T" --body "sandboxed send for ack" --require-ack "$B" | python3 -c "import json,sys;print(json.load(sys.stdin)['result']['data'])")
echo "message for ack: $MSG"; echo
sbx ch-setup "$S/ht-b" check-in --lifecycle-event probe-b1
sbx ch-setup "$S/ht-b" ack "$MSG"
sbx ch-setup "$S/ht-b" accept "$T"
echo "# control: inviting an already joined seat fails the same way outside the sandbox (not a sandbox effect)"
un "$S/ht-a" invite "$T" --seat "$B"
sbx ch-setup "$S/ht-b" send "$T" --body "sandboxed reply from B"
sbx ch-setup "$S/ht-b" check-in
sbx ch-setup "$S/ht-b" leave "$T"
sbx ch-setup "$S/ht-a" invite "$T" --seat "$B"
echo "# client journals now: $(cd "$I" && find intents contexts -maxdepth 1 | sort | tr '\n' ' ')"
echo

echo "## N after, negatives in the same sandbox: everything outside the two roots stays read-only"
sbx ch-setup touch "$I/x"
sbx ch-setup touch "$I/threads.sqlite3"
sbx ch-setup python3 "$HERE/try-write.py" open-rw "$I/threads.sqlite3"
sbx ch-setup python3 "$HERE/try-write.py" sqlite "$I/threads.sqlite3"
sbx ch-setup python3 "$HERE/try-write.py" append "$I/threads.sqlite3-wal"
sbx ch-setup python3 "$HERE/try-write.py" append "$I/threads.sqlite3-shm"
sbx ch-setup python3 "$HERE/try-write.py" open-rw "$I/endpoint.json"
sbx ch-setup python3 "$HERE/try-write.py" open-rw "$I/owner.lock"
sbx ch-setup python3 "$HERE/try-write.py" append "$I/daemon.log"
sbx ch-setup touch "$I/../x"
sbx ch-setup touch "$ST/x"
sbx ch-setup touch "$ST/setup/x"
sbx ch-setup mkdir "$I/client2"
sbx ch-setup ln -s "$I/threads.sqlite3" "$I/contexts/esc"
sbx ch-setup python3 "$HERE/try-write.py" open-rw "$I/contexts/esc"
sbx ch-setup mv "$I/intents/journal-format" "$I/moved-out"
sbx ch-setup touch "$S/x"
sbx ch-setup touch "$HOME/x"
sbx ch-setup touch "$S/ch-setup/config.toml"
echo "## N2 positive controls for the roots themselves"
sbx ch-setup touch "$I/intents/probe-ok" "$I/contexts/probe-ok"
sbx ch-setup python3 "$HERE/try-write.py" append "$I/intents/probe-ok"
rm -f "$I/intents/probe-ok" "$I/contexts/probe-ok" "$I/contexts/esc"
python3 -c "import sqlite3;print('# DB has probe_x table:', bool(sqlite3.connect('file:$I/threads.sqlite3?mode=ro',uri=True).execute(\"select 1 from sqlite_master where name='probe_x'\").fetchone()))"

base daemon stop >/dev/null 2>&1
kill $FHPID 2>/dev/null
