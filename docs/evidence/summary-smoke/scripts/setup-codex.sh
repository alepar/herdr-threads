#!/bin/sh
export CODEX_HOME=/private/tmp/ht-summary-smoke/codex-home
export PATH=/private/tmp/ht-summary-smoke/bin:$PATH
exec /private/tmp/ht-summary-smoke/ht "$@"
