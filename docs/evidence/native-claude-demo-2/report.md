# Native Claude demo 2 (ht-4is.11.4, ht-910 Claude half): unhinted acceptance run

Date: 2026-09-30. Public `setup claude --project` CLI, the driver's default prompt, **no** hint. Claude Code 2.1.285, print mode, Haiku 4.5, `--permission-mode default`. Binary from integration `72a86c4` (sha256 `f8e2473f7f71ad254dc4d6268bee5957d11b3fd194f08c7b2313f045d9b257ff`). 1 of 5 launches used, **$0.0259**. The report was written by the demo agent and saved by the coordinator.

## Verdict: FAIL (blocked by product defect P6)

| # | Item | Result |
|---|---|---|
| A1 | Prelaunch invite and require-ACK handoff; nothing ACKed before launch | **PASS** |
| A2 | Hook output reaches the model | **PASS**. SessionStart delivered 2548 bytes naming the thread, the message and the ready commands. PreToolUse made 0 deliveries, which is correct because nothing changed (the probe was quiet). |
| A3 | Top-level model accepts and ACKs the exact message ID unaided | **FAIL**. The model ran the exact ready command `herdr-threads --state-dir … read <thread> --recent 20` with no hint, so **P1 is fixed**. Claude print mode denied it ("This command requires approval"), and the model stopped. |
| A4 | Provenance | No accept or ACK was stored. The lifecycle binding is `cooperative_top_level`. |
| A5 / A6 | Child ACK / restart and resume | Not run (BLOCKED on S18) |
| Flapping | Availability | **None**. The agent seat stayed at `unavailability_episode=1` with one decision and 0 warnings over about 40 s. P2 and P3 look fixed; the window was short. |

## Defects

- **P6 (product, High).** `setup claude` installs only `Bash(export HERDR_THREADS_CALLER_CONTEXT=*)` (`src/harness/claude.rs:342`, `src/cli/setup.rs:58-63`, `src/harness/setup.rs:71`). The production hook never rewrites commands (`src/cli/hook.rs:329-339`; `encode_tool_response` is called only from tests), so that rule is dead. Nothing allows the `herdr-threads …` commands the hook tells the model to run. **Repro:** run setup, then `claude -p --permission-mode default` in a pane with pending mail; the stream shows `system/permission_denied` (`run1-unhinted/claude-initial-aab6.stream.jsonl`, event 12). **User decision:** setup installs the owned rule `Bash(herdr-threads *)` and drops the export rule.
- **D4 (driver, High).** `transcript_calls` (`scripts/validate-native-demo.py:741-742`) crashes on the string `message` of a `permission_denied` event, which hides the real S18 reasons.
- **D5 (driver, Low).** `manifest_source` says `cooperative` even though no provenance was stored.
- **D6 (driver, Low).** The setup check still requires the export rule (lines 490-492), and there is no check that the ready commands are permitted.

## Not run

A diagnostic run with `Bash(herdr-threads *)` added was refused by the session's permission classifier and was not attempted by other means. After P6 is fixed, the full initial/restart/resume run should be repeated with a longer window.

## Hygiene

The owned tabs are closed and the daemons are stopped. The Herdr server was not restarted. `~/.claude/settings.json` is unchanged. Claude left one transcript directory under `~/.claude/projects`.

## Evidence

`run1-unhinted/`, `dry-run-baseline/`, `driver-logs/live1.log`, `cost.json`. Redaction: `~`, `$SCRATCH` and `<user>` stand in for local values; SQLite snapshots are replaced by JSON extracts.
