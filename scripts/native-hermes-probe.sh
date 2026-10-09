#!/bin/sh
# Explicit interpreter: no guessed native Python/launcher/bootstrap.
set -eu
: "${HT_PROBE_PYTHON:?set HT_PROBE_PYTHON to the explicit driver Python interpreter}"
exec "$HT_PROBE_PYTHON" -I -B "$(dirname "$0")/native-hermes-probe.py" "$@"
