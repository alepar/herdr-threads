# Native matrix smoke (ht-p03.38) — early run, not closure evidence

Each cell ran once (retries listed under Notes) through `scripts/validate-native-demo.sh --cell <name>`, in its own
isolated named Herdr session (private server under `/tmp/ih.XXXXXX` from `scripts/lib/isolated-herdr.sh`; the shared
Herdr server was never started, stopped or reconfigured, and its workspace list was unchanged before and after).
Tree SHA of every run: `7590dfc530c8f164f7cf21233d2b59cbd781846b` (integration tip plus stack parent ht-p03.1),
binary `target/debug/herdr-threads` built from that tree. Judged by the hardened validator (ht-p03.18).
Claude cells ran Claude Code 2.1.286 from the isolated npm prefix `/private/tmp/ht-cell-claude-2.1.286`; the installed
2.1.287 and `~/.claude` were not changed (the driver's existing read-only credential use and its approved
`~/.claude.json` record for TUI trust are the only touches). Codex: `codex-cli 0.159.3`, default profile.
Manifests live in the run roots below (not committed).

| Cell | Harness before / after | Outcome | Manifest (run root evidence dir) | Bucket bead (FAIL) |
|---|---|---|---|---|
| codex-manual | codex-cli 0.159.3 / 0.159.3 | PASS | <tmp>/ht-native-demo/ht-demo-codex-20261001T205051-3d242b/evidence | |
| codex-managed | codex-cli 0.159.3 / 0.159.3 | PASS | <tmp>/ht-native-demo/ht-demo-codex-20261001T205122-5003b1/evidence | |
| claude-manual | 2.1.286 / 2.1.286 | PASS | <tmp>/ht-native-demo/ht-demo-claude-20261001T205023-0d674e/evidence | |
| claude-managed | 2.1.286 / 2.1.286 | PASS | <tmp>/ht-native-demo/ht-demo-claude-20261001T205212-00ac60/evidence | |
| codex-no-initial-prompt | codex-cli 0.159.3 / 0.159.3 | FAIL: Codex TUI shows a "Trust this folder?" screen despite `-s/-a`; nothing typed (S17) | /private/tmp/ht-native-demo/ht-demo-codex-20261001T205249-6205e2/evidence | ht-p03.20 |
| claude-no-initial-prompt | 2.1.286 / 2.1.286 | PASS (idle recovery wake outcome `submitted`, ACK observed) | /private/tmp/ht-native-demo/ht-demo-claude-20261001T210714-1edf8d/evidence | |
| children-claude | 2.1.286 / 2.1.286 | PASS (SK1 2 concurrent subagents, SK2 no child write, SK3 root ACK) | <tmp>/ht-native-demo/ht-demo-claude-20261001T205747-979b58/evidence | |
| children-codex | codex-cli 0.159.3 / 0.159.3 | PASS (SK1-SK3) | <tmp>/ht-native-demo/ht-demo-codex-20261001T205833-c70e73/evidence | |
| sw2-claude | 2.1.286 / 2.1.286 | NOT_EXERCISED: SW1 PASS, SW2 not owed because the warning was already offered at a verified check-in (`offered_through_seq 13`), so no coalesced wake happened | /private/tmp/ht-native-demo/ht-demo-claude-20261001T210930-863d8b/evidence | |
| sw2-codex | codex-cli 0.159.3 / 0.159.3 | FAIL: Codex TUI trust screen (S17), SW2 never reached | /private/tmp/ht-native-demo/ht-demo-codex-20261001T210039-76b0cf/evidence | ht-p03.20 |
| codex-tui-children-write-absence | codex-cli 0.159.3 / 0.159.3 | FAIL: Codex TUI trust screen (S17), SK1-SK3 never reached | /private/tmp/ht-native-demo/ht-demo-codex-20261001T210049-ed6ddd/evidence | ht-p03.20 |
| ht910-claude | 2.1.286 / 2.1.286 | PASS (daemon stopped and re-ensured, agent stayed in its pane, post-restart handoff ACKed by the same agent) | /private/tmp/ht-native-demo/ht-demo-claude-20261001T211234-6d889a/evidence | |
| ht910-codex | codex-cli 0.159.3 / 0.159.3 | FAIL: Codex TUI trust screen (S17), daemon-restart phase never reached | /private/tmp/ht-native-demo/ht-demo-codex-20261001T210132-a0b5a2/evidence | ht-p03.20 |
| p40-crash-fix3 | codex-cli 0.159.3 / 0.159.3 | FAIL: Codex TUI trust screen (S17), daemon-crash phase never reached | /private/tmp/ht-native-demo/ht-demo-codex-20261001T210141-ebf3b0/evidence | ht-p03.20 |
| codex-sandbox-xdg-state | codex-cli 0.159.3 / 0.159.3 | PASS (state dir `<out>/home/.local/state/herdr-threads`, model ACK from inside the workspace-write sandbox) | <tmp>/ht-native-demo/ht-demo-codex-20261001T210150-d7118b/evidence | |
| wave28-claude-wake-submission | 2.1.286 / 2.1.286 | PASS (SL1 wake outcome `submitted`, SL2 marker once, ACK observed) | /private/tmp/ht-native-demo/ht-demo-claude-20261001T210823-cc7136/evidence | |
| claude-installed-2.1.287 | 2.1.287 installed, not run | NOT_EXERCISED-pending-admission (refused until Optimistic admission, ht-p03.13; run by ht-p03.20) | | |

## Notes

- Every PASS is a cooperative-claim PASS (`cooperative_top_level`), exactly as the validator labels it; none is native proof.
- Runner defect fixed here: the first Claude TUI runs (claude-no-initial-prompt, wave28, sw2-claude, ht910-claude) were invalid
  because the private server inherited `CLAUDE_CODE_CHILD_SESSION` from the launching Claude Code session, which turns
  transcript saving off in the pane (S17H/S18 "transcript not found"). The cell runner now unsets `CLAUDE_CODE_*`, `CLAUDECODE`,
  `CLAUDE_PID`, `CLAUDE_EFFORT` for the private server; the rows above are the reruns. The invalid first runs are not counted.
- ht910-claude: one rerun more. First valid run failed at S08 `seat resolve` with `stale_host_observation` (transient; the next
  run PASSed). ht910/P40 cells carry `--verify-wait 150` because the post-restart wake waits out the minimum wake delay.
- The five Codex TUI cells all fail at the same point: codex-cli 0.159.3 prints its folder-trust screen even with explicit
  `-s workspace-write -a on-request`, and the driver refuses to answer it (answering persists trust into CODEX_HOME). This is a
  harness/validator infrastructure blocker for every Codex TUI cell, so their scenario logic is UNMEASURED here. Filed on ht-p03.20.
- Cell mappings the driver could not express directly: `ht910-*` and `p40-crash-fix3` use new minimal driver phases
  `daemon-restart` (`daemon stop` + `daemon ensure`) and `daemon-crash` (SIGKILL of the process holding the daemon socket +
  `daemon ensure`), TUI only; p40-crash-fix3 as "daemon crash and reconnect with Codex agent staying in its pane" is an
  interpretation. `codex-sandbox-xdg-state` is `--state-dir` at the XDG default layout under the cell's own HOME-like dir.
  `wave28-claude-wake-submission` reuses the `lostprompt` TUI scenario; `*-no-initial-prompt` is `lostprompt`.

## Coverage check

`for c in $(scripts/validate-native-demo.sh --cell list)` against the table rows (17 cells, 0 missing):

```
row ok: codex-manual
row ok: codex-managed
row ok: claude-manual
row ok: claude-managed
row ok: codex-no-initial-prompt
row ok: claude-no-initial-prompt
row ok: children-claude
row ok: children-codex
row ok: sw2-claude
row ok: sw2-codex
row ok: codex-tui-children-write-absence
row ok: ht910-claude
row ok: ht910-codex
row ok: p40-crash-fix3
row ok: codex-sandbox-xdg-state
row ok: wave28-claude-wake-submission
row ok: claude-installed-2.1.287
```
