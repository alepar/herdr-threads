#!/bin/bash
# usage: cap.sh <harness> <label> : saves visible/recent/detection agent reads + agent get/explain
# into captures/<harness>-<label>.*, redacting the home directory and user name.
H=$1; L=$2
case $H in claude) A=spk-claude;; codex) A=spk-codex;; esac
D="$(cd "$(dirname "$0")/.." && pwd)/captures"
red() { sed -e "s#$HOME#~#g" -e 's#alepar@Mac#user@host#g' -e 's#alepar#user#g'; }
for s in visible recent detection; do
  herdr agent read $A --source $s 2>&1 | red > "$D/$H-$L.read-$s.txt"
done
herdr agent get $A 2>&1 | red > "$D/$H-$L.get.json"
herdr agent explain $A --format text 2>&1 | red > "$D/$H-$L.explain.txt"
