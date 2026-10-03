# Summary and poke native smoke (ht-1ip.15)

Overall result: PARTIAL. Recovery hook text, parallel summary workers, shared block reuse by a second seat and
catch-up hold were demonstrated on both harnesses. The soft-deadline poke was only observed incidentally (not in
a controlled probe), and the skip on a focused pane was NOT demonstrated on either harness. All capture paths
below are relative to `captures/`. Home paths are redacted.

## Versions and setup

| Item | Value |
| --- | --- |
| herdr-threads | 0.1.0 (protocol 2), release binary built from branch `task-ht-1ip.15`, base `ad1f5739` (stack of ht-1ip.14). The exact binary commit was not recorded. |
| herdr | 0.9.1 |
| claude | 2.1.287, recipe `claude-hooks-2.1.287` (compaction recovery supported), model `claude-haiku-4-5-20251001` (`state/doctor.txt`) |
| codex | 0.160.0, recipe `codex-hooks-v1` (schema-matched, live-unverified), model `gpt-5.6-luna`, effort low (`state/doctor.txt`, `scripts/launch-codex.sh`) |
| Isolation | private Herdr server and private herdr-threads state dir under `/private/tmp/ht-summary-smoke/`; project-local hooks in scratch project dirs (`state/claude-scratch-settings.json`, `state/codex-scratch-hooks.json`). `state/doctor.txt` shows the user's global `~/.claude/settings.json` and `~/.codex/hooks.json` have no herdr-threads hooks: they were not touched. |
| Codex account | scratch `CODEX_HOME` seeded from the `protonmail` aisw profile (config.toml copied, auth.json symlinked and never read, `scripts/mk-codex-home.sh`); the global active account was not switched. Setup edits: `state/codex-scratch-config.before-setup.toml` vs `state/codex-scratch-config.final.toml`. The profile directory itself was not modified. |
| Claude settings | project/local sources only, `promptSuggestionEnabled=false` (workaround for ht-jf3), see `scripts/launch-claude.sh` |
| Instance settings | `state/instance-settings.json`: chunk_bytes 2048, display_bytes 8192, p99_cold_ms 120000 (brief asked 20000), exit_grace_ms 15000. Receipt deadlines 300-400 s in the poke probes (brief asked 180 s). |

## Per-harness results

Claude: X = seat-tJDlgi2m, thread thread-I2SWujnv. Codex: X = seat-RJ7yryH6, thread thread-DWKSdAn7. Y is the second agent seat of the same harness.

| Step | Claude | Codex |
| --- | --- | --- |
| Recovery event hook text (instruction plus thread as hot) | PASS: compact, SessionStart(compact) additionalContext in `claude-x-01-hook-after-recovery.txt`; pane after: `claude-x-after-compact.pane.txt` | PASS: compact, developer message in `codex-x-01-hook-after-recovery.txt` |
| Summary procedure, parallel workers, to Ready | PASS with a caveat: parallel Haiku workers (`claude-x-03-worker-spawns-and-submits.txt`), blocks served in `claude-x-04-ready.txt`. The first attempt failed (`claude-x-attempt1-no-skill.pane.txt`: workers invented JSON shapes) because the hook text names a skill setup does not install, bead ht-dtq. The passing run came only after the operator told X to run `herdr-threads skill`. 36 `fallback` lines remain in the served blocks, so not every block has a narrative. | PASS with a caveat: parallel workers (spawn_agent, low effort) in `codex-x-03-worker-spawns-and-submits.txt`, blocks in `codex-x-04-ready.txt`; 12 `fallback` lines remain. |
| Shared block reuse by second seat Y | PASS: Y's `summary` returns the same 14 block ids as X (`claude-y-04-ready.txt` vs `claude-x-04-ready.txt`, ids compared offline); no worker rerun (`claude-y-03-worker-spawns-and-submits.txt`) | PASS: same 13 block ids (`codex-y-04-ready.txt` vs `codex-x-04-ready.txt`) |
| Catch-up hold and extension on a pending receipt | PASS for the hold: `pending-receipts` lists 51 rows as `deferred: recipient catching up (until ...)` with `deferred_until` after the original deadline (`claude-catchup/pending-receipts-X.txt`, `deferred-rows.txt`). Not captured separately: X's attention not offering the extra require-ACK message. | PASS for the hold: 43 deferred rows (`codex-catchup/`). Same gap on the attention-offer check. |
| Soft-deadline poke on idle unfocused agent | PARTIAL; controlled probes FAILED: `claude-poke/` and `claude-poke-run1-starved-by-wake-backoff/` show no poke before the deadline (starved by the wake retry backoff, bead ht-2i4); in `claude-poke-run2-agents-acked-at-first-wake/` the agents ACKed at the first wake, so no poke was due. Incidental real pokes: `receipt due in 18s` and `58s` reached Y at 23:37:53 and 23:38:23 (`pokes-and-wakes-received.txt`). | PARTIAL, same shape: `codex-poke-run1-focus-not-applied/`, `codex-poke-after-release*/` show no poke in the probe window (ht-2i4). Incidental real pokes: `receipt due in ...` to X at 00:07:35, 00:08:24, 00:19:29, 00:20:08 and to Y (`pokes-and-wakes-received.txt`). |
| Skip on a focused pane, then poke after focus moves away | VERIFIED on Claude 2.1.288, 2026-10-03, on the real Herdr session (see [focus-probe/](focus-probe/report.md)): no poke while focused, poke after focus moved away. Original private-server attempt: `herdr agent focus` ran at T+74 / T+101 (`claude-poke*/events.txt`) but every sample reads `focused=false` (`claude-poke*/samples.txt`), so the pane was never focused; no skip log line. Bead ht-yuz. | NOT VERIFIED (acceptance unmet): focus events in `codex-poke-run1-focus-not-applied/events.txt` and `codex-poke-after-release*/events.txt`; samples show `focused=false`. Bead ht-yuz. |

The incidental pokes show the daemon can deliver a poke prompt on both harnesses. They are not the controlled
soft-point-on-idle-unfocused demonstration the brief asked for, and nothing here proves the skip logic ran.

## Findings filed

- ht-jf3: Claude prompt-suggestion ghost text read as a typed draft; wakes and pokes go `unsafe` and back off 5 min (`claude-x-after-compact.pane.txt`, `claude-poke-run1-starved-by-wake-backoff/`).
- ht-dtq: reset hook text names a skill setup does not install (`claude-x-attempt1-no-skill.pane.txt`).
- ht-2i4: soft poke starved by the wake retry backoff (`claude-poke/`, `codex-poke*/`).
- ht-yuz: focus never applied on the private Herdr server, so the focused-pane skip is unproven. Cause not established (a headless server with no attached client is a hypothesis only).

## Cost

- Claude (Haiku 4.5): 39 transcripts, 3,064 input / 63,353 output / 15,923,164 cache-read / 843,637 cache-write tokens, about $2.97 at Haiku 4.5 list price (`usage.txt`; an estimate, not a bill).
- Codex (`gpt-5.6-luna`): 25 rollouts, 11,237,026 input (10,516,224 cached) / 54,416 output tokens; no price assumed (`usage.txt`).
- The $3 budget was consumed by the Claude side, so the focus half was not re-run.

## Not verified

- Skip on a focused pane on Codex (Claude verified in [focus-probe/](focus-probe/report.md)).
- A controlled soft-point poke on an idle unfocused agent, both harnesses.
- That X's attention output omits the catch-up-held message (only the `pending-receipts` deferral line is captured).
- Codex hooks are `schema-matched, live-unverified` per the daemon's own limitation line (`state/doctor.txt`).
- Native receipt and execution verification: harness mode is `cooperative` on both.
- The exact commit of the binary used.

## Integrity

Run `shasum -a 256 -c SHA256SUMS` in this directory. It lists every file except itself.
