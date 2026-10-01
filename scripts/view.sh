#!/bin/sh
set -eu

: "${HERDR_PLUGIN_STATE_DIR:?Herdr plugin state directory is required}"
: "${HERDR_SOCKET_PATH:?Herdr socket path is required}"
package_root=$(CDPATH= cd "$(dirname "$0")/.." && pwd -P)
binary="$package_root/bin/herdr-threads"

case "${1:-}" in
    open)
        : "${HERDR_BIN_PATH:?Herdr binary path is required}"
        exec "$HERDR_BIN_PATH" plugin pane open --plugin herdr-threads --entrypoint operator
        ;;
    ensure) exec "$binary" --state-dir "$HERDR_PLUGIN_STATE_DIR" --host-endpoint "$HERDR_SOCKET_PATH" daemon ensure ;;
    health) exec "$binary" --state-dir "$HERDR_PLUGIN_STATE_DIR" --host-endpoint "$HERDR_SOCKET_PATH" daemon health ;;
    doctor) exec "$binary" --state-dir "$HERDR_PLUGIN_STATE_DIR" --host-endpoint "$HERDR_SOCKET_PATH" doctor ;;
    stop) exec "$binary" --state-dir "$HERDR_PLUGIN_STATE_DIR" --host-endpoint "$HERDR_SOCKET_PATH" daemon stop ;;
    view)
        while :; do
            "$binary" --state-dir "$HERDR_PLUGIN_STATE_DIR" --host-endpoint "$HERDR_SOCKET_PATH" view --once
            while :; do
                printf '\nEnter to refresh, q to quit: '
                if ! IFS= read -r answer; then
                    exit 0
                fi
                case "$answer" in
                    q|Q) exit 0 ;;
                    '') break ;;
                    *) printf '%s\n' 'Press Enter to refresh or q to quit.' ;;
                esac
            done
        done
        ;;
    *) printf '%s\n' 'usage: view.sh {open|ensure|health|doctor|stop|view}' >&2; exit 2 ;;
esac
