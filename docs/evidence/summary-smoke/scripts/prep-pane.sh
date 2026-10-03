#!/bin/sh
# usage: prep-pane.sh PANE ...   put the smoke bin dir first in the pane shell's PATH
for p in "$@"; do
  /private/tmp/ht-summary-smoke/h pane run "$p" 'export PATH=/private/tmp/ht-summary-smoke/bin:$PATH DISABLE_AUTOUPDATER=1; clear' >/dev/null
done
