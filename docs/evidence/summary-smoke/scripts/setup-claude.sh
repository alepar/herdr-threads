#!/bin/sh
export CLAUDE_CONFIG_DIR=/private/tmp/ht-summary-smoke/claude-config
export PATH=/private/tmp/ht-summary-smoke/bin:$PATH
exec /private/tmp/ht-summary-smoke/ht "$@"
