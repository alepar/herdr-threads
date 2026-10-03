#!/bin/sh
# Private herdr server for the summary smoke. HOME stays the real one (harness login); config and socket are private.
# Every marker the coordinating Claude session exported is dropped so panes start as fresh top-level sessions.
unset HERDR_ENV HERDR_PANE_ID HERDR_TAB_ID HERDR_WORKSPACE_ID HERDR_AGENT HERDR_BIN_PATH
unset AISW_SHELL_HOOK CLAUDE_CODE_ENTRYPOINT CLAUDE_CODE_MESSAGING_SOCKET CLAUDE_CODE_MESSAGING_TOKEN CLAUDE_CODE_EXECPATH
unset CLAUDECODE CLAUDE_CODE_SESSION_ID CLAUDE_CODE_CHILD_SESSION CLAUDE_CODE_SESSION_ATTENDED CLAUDE_PID CLAUDE_EFFORT
export HERDR_CONFIG_PATH=/private/tmp/ht-summary-smoke/cfg/herdr.toml
export HERDR_SOCKET_PATH=/private/tmp/ht-summary-smoke/h.sock
export PATH=/private/tmp/ht-summary-smoke/bin:$PATH
export DISABLE_AUTOUPDATER=1 HERDR_PLUGIN_STATE_DIR=/private/tmp/ht-summary-smoke/state
export XDG_CONFIG_HOME=/private/tmp/ht-summary-smoke/xcfg XDG_STATE_HOME=/private/tmp/ht-summary-smoke/xst XDG_RUNTIME_DIR=/private/tmp/ht-summary-smoke/xrt
rm -f "$HERDR_SOCKET_PATH"
exec /Users/USER/.local/bin/herdr server
