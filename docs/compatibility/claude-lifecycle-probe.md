# Claude lifecycle probe: installed startup, clear and resume

On 2026-09-28, installed Claude Code 2.1.283 and Herdr 0.9.1 were tested in one owned private Herdr server/pane. Startup stopped at Claude's native workspace trust prompt before a lifecycle capture was produced. The default was `No, exit`; the driver did not answer it. Clear, resume, and compact were not attempted. No installed lifecycle parser fields or duplicate-event identity are qualified by this run.

The source is `tests/native/cooperative/claude_lifecycle/{run,capture}.py`. `run.py prepare RUN_DIR` reuses the existing NativeFixture ownership/private-directory helpers. It copies permission rules and the minimum existing native auth/config state into private configuration. Global settings and auth/config files are never written. The harmless private baseline SessionStart sentinel and an additional `--settings` lifecycle capture overlay are candidates for composition measurement. Their execution was not observed, so settings bytes do not establish composition support. Existing global plugins/hooks are not executed by this fixture.

`run.py server RUN_DIR` holds the isolated server in the foreground. `run.py cli RUN_DIR ...` addresses only its private socket. Verify the newly created pane's inherited environment from a non-login shell before launching Claude. This run observed `HERDR_ENV=1`, workspace `w1`, tab `w1:t1`, and pane `w1:p1`. The finite launch was:

```text
claude --model sonnet --effort low --settings <RUN_DIR>/config/overlay.json --tools '' --strict-mcp-config --mcp-config '{"mcpServers":{}}'
```

Keep `HCOM_AUTO_APPROVE=0` and `HCOM_AUTO_TRUST_WORKSPACE=0`; no approval or sandbox bypass flag is used. `run.py cleanup RUN_DIR` closes only ledger-recorded resources. Old server PIDs must be absent; ambiguous live old PIDs are retained. Source scripts emit no hook decisions, accept, or ACK operations. The capture records only event/source/session/agent metadata, event-identity presence, timing, version, and caller pane context; it does not retain transcript paths, tool input, environment dumps, or secrets.

The sandbox denied the first server and private socket connection. Scoped escalation started the same private fixture server successfully. An initially detached server exited with its execution session; the final server used the foreground runner. Cleanup completed all ledger transitions, and the foreground server command exited 0. Hashes of global `.claude/settings.json`, `.claude/settings.local.json`, and `.claude.json` were identical before and after. Python compilation and all 13 shared fixture tests passed.

Private sanitized evidence is in `.superpowers/sdd/ht-4is-plan/claude-lifecycle-capture/`, with sibling report/evidence files. Official [SessionStart documentation](https://code.claude.com/docs/en/hooks#sessionstart) describes lifecycle sources; those documented declarations remain separate from this blocked installed observation.

## Attempt 2: existing repository trust preserved

The authorized continuation used `~/code/herdr-threads`, whose exact existing `hasTrustDialogAccepted=true` decision was checked and privately mirrored. No other project trust decisions or global plugins/hooks were copied. Neither project settings file exists, and `--setting-sources user` limited configuration to the private user baseline plus `--settings` overlay. Permission rules and the checked global file hashes stayed unchanged. `prepare-trusted` provides this bounded setup; its final source avoids temporarily copying unrelated project entries.

Interactive Claude Code 2.1.283 emitted these actual rows:

| Native action | SessionStart source | Native session | Hook parent PID |
| --- | --- | --- | --- |
| Fresh launch | startup | 406c6097-796d-4585-bedb-519986dd577d | 51473 |
| `/clear` | clear | cc5ee229-f421-4be8-877a-73b68ea785a7 | 51473 |
| Fresh process `--resume` of original session | resume | 406c6097-796d-4585-bedb-519986dd577d | 52394 |

Both the harmless baseline sentinel and capture overlay executed for each source. This establishes composition for those private hooks and configurations. Their ordering varied at resume. Captured native fields were `hook_event_name`, `source`, and `session_id`; `agent_id`, `agent_type`, `event_id`, and `hook_event_id` were absent. No duplicate delivery/retry identity is established. Hook timestamps and parent PIDs are observer metadata, not native lifecycle IDs.

The first short prompt attempt returned `Not logged in` in the private configuration; the host wait reported `agent_prompt_stalled`. No successful model response, token retrieval, login, approval answer, or model-issued operation occurred. `/clear`, `/exit`, and `--resume` still emitted lifecycle metadata without authentication. Compact was not attempted. No claim about compaction, fork, child starts, hook output delivery to a model, or receipt authority follows.

Attempt1 artifacts and commit `5db2d50` remain unchanged. Attempt2 sanitized rows, commands, allowlisted blocker UI, ledger, versions and hashes are in `claude-lifecycle-capture/attempt2/` with distinct `claude-lifecycle-capture-attempt2-{report.md,evidence.json}`. All attempt2 resources were cleaned; the private server foreground command exited0. Source checkpoints require root review before integration.

## Runner correction after source review I1

The runner now accepts a finite command grammar, rather than passing arbitrary Herdr CLI arguments through. Server creation and cleanup use their dedicated actions; all non-CLI actions reject surplus arguments before dispatch. CLI commands are workspace list/create, owned pane get/run, and owned pane agent read/prompt/start. Session management, alternate endpoint selectors (including equals forms), unknown flags and extra arguments are rejected before subprocess. Workspace creation records the returned root pane in the fixture ledger; malformed successful responses retain the response and require inspection without retry.

Mutation targets must be exact currently owned pane IDs; agent names do not serve as mutation targets. Pane commands and prompts use runner syntax `pane run PANE -- PAYLOAD` and `agent prompt PANE [--wait --timeout MS] -- PAYLOAD`. The runner consumes that delimiter and keeps the payload as one literal argument. Herdr0.9.1 does not consume a delimiter in its pane-run or prompt parser; option-shaped payloads therefore fail before subprocess instead of becoming endpoint selectors. Shell/path spacing is preserved. The native start tail is restricted to the recorded Sonnet/low/private-overlay/user-settings/no-tools/empty-MCP recipe, optionally followed by a UUID resume argument after Herdr's actual native delimiter. All CLI subprocesses retain a60second bound; waits are at most60000ms and reads at most120lines.

This source correction was checked only with an inert subprocess stub and temporary fixture ledgers. Historical attempts, native rows and `capture.py` remain byte-identical; no new lifecycle capture or endpoint contact occurred.
