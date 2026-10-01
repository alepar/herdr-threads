#!/bin/sh
# Codex sandbox default-deny probe (no model). Set SCRATCH (holding conn.py, listen.py, net.sh, a scratch state/ with a running daemon, fakehost.sock and neg.sock listeners) and TARGET (cargo target dir).
S=$SCRATCH
H=$TARGET/debug/herdr-threads
CODEX=~/.local/bin/codex
DSOCK=$(python3 -c "import json,glob;print(json.load(open(glob.glob('$S/state/instances/*/endpoint.json')[0]))['endpoint'])")
HSOCK=$HOME/.config/herdr/herdr.sock
FAKEHOST=$S/fakehost.sock
CH0=$S/codex-home-empty; CH1=$S/codex-home-allowance
rm -rf $CH0 $CH1; mkdir -p $CH0 $CH1
cat > $CH1/config.toml <<TOML
[sandbox_workspace_write]
network_access = true

[features.network_proxy]
enabled = true
unix_sockets = { "$DSOCK" = "allow" }
TOML
ALLOW="-c sandbox_workspace_write.network_access=true -c features.network_proxy.enabled=true"
UNIX="features.network_proxy.unix_sockets={\"$DSOCK\"=\"allow\"}"
HC="$H --state-dir $S/state --host-endpoint $FAKEHOST daemon health"
run() { echo "\$ $*"; timeout 60 "$@" 2>&1; echo "rc=$?"; echo; }
echo "# Codex $($CODEX --version) sandbox default-deny probe (no model), $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "# codex binary: $(python3 -c "import os;print(os.path.realpath('$CODEX'))") sha256 $(shasum -a 256 $(python3 -c "import os;print(os.path.realpath('$CODEX'))") | cut -c1-64)"
echo "# DSOCK=$DSOCK (stable path of scratch daemon; state=$S/state host=$FAKEHOST fake). NEG=$S/neg.sock; TCP 127.0.0.1:47812 owned listener; HSOCK=$HSOCK (connect+close only)"
echo "# CH0=$CH0 (empty CODEX_HOME); CH1=$CH1 (CODEX_HOME holding only the allowance):"
sed 's/^/#   /' $CH1/config.toml
echo
echo "## P0 unsandboxed control"; run sh $S/net.sh $S $DSOCK $HSOCK
echo "## P0b unsandboxed daemon health"; echo "\$ $HC"; $HC >/dev/null 2>&1; echo "rc=$?"; echo
echo "## P1 workspace-write, empty CODEX_HOME, no allowance: product CLI"; CODEX_HOME=$CH0 run $CODEX sandbox -c 'sandbox_mode="workspace-write"' -- $HC
echo "## P1b workspace-write, empty CODEX_HOME, no allowance: net.sh"; CODEX_HOME=$CH0 run $CODEX sandbox -c 'sandbox_mode="workspace-write"' -- sh $S/net.sh $S $DSOCK $HSOCK
echo "## P2 workspace-write + allowance as -c overrides (session-scoped form): net.sh"; CODEX_HOME=$CH0 run $CODEX sandbox -c 'sandbox_mode="workspace-write"' $ALLOW -c "$UNIX" -- sh $S/net.sh $S $DSOCK $HSOCK
echo "## P2b -c allowance: product CLI"; echo "\$ ... -- $HC | head -c 200"; CODEX_HOME=$CH0 timeout 60 $CODEX sandbox -c 'sandbox_mode="workspace-write"' $ALLOW -c "$UNIX" -- $HC 2>&1 | head -c 200; echo; echo
echo "## P2c -c allowance: curl must fail"; CODEX_HOME=$CH0 run $CODEX sandbox -c 'sandbox_mode="workspace-write"' $ALLOW -c "$UNIX" -- curl -sS -m 8 https://example.com -o /dev/null
echo "## P3 workspace-write, CODEX_HOME holding only the allowance in config.toml (user-level setup form): net.sh"; CODEX_HOME=$CH1 run $CODEX sandbox -c 'sandbox_mode="workspace-write"' -- sh $S/net.sh $S $DSOCK $HSOCK
echo "## P3b config.toml allowance: product CLI"; echo "\$ ... -- $HC | head -c 200"; CODEX_HOME=$CH1 timeout 60 $CODEX sandbox -c 'sandbox_mode="workspace-write"' -- $HC 2>&1 | head -c 200; echo; echo
echo "## P3c config.toml allowance: curl must fail"; CODEX_HOME=$CH1 run $CODEX sandbox -c 'sandbox_mode="workspace-write"' -- curl -sS -m 8 https://example.com -o /dev/null
echo "## P3d config.toml allowance: curl --noproxy must fail"; CODEX_HOME=$CH1 run $CODEX sandbox -c 'sandbox_mode="workspace-write"' -- curl -sS -m 8 --noproxy '*' https://example.com -o /dev/null
