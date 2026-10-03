#!/bin/sh
# Scratch CODEX_HOME for the smoke: the protonmail profile's config.toml (copied, plus trust for the scratch project)
# and its auth.json by symlink (never read). The user's profile directory is not modified.
P=/Users/USER/.aisw/profiles/codex/protonmail
C=/private/tmp/ht-summary-smoke/codex-home
mkdir -p "$C"
cp "$P/config.toml" "$C/config.toml"
printf '\n[projects."/private/tmp/ht-summary-smoke/proj-codex"]\ntrust_level = "trusted"\n' >> "$C/config.toml"
ln -sf "$P/auth.json" "$C/auth.json"
chmod 600 "$C/config.toml"
ls -la "$C"
