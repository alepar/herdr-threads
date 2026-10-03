# Harness transcripts as the version source (ht-xoc.8)

Evidence for the transcript-based attribution in
`docs/superpowers/specs/2026-10-02-harness-version-evidence-design.md` (Attribution). Captured 2026-10-02.

## Claude Code (2.1.288, and 2.1.250 installed under a prefix)

Runs with `claude --settings <scratch>` (capture hook), interactive in a scratch Herdr pane and in `-p` mode; no
harness configuration edited.

| Question | Finding |
| --- | --- |
| Transcript at SessionStart (startup, fork) | Absent. Interactive: created about 12 s later, at the first prompt. |
| Transcript at the first PreToolUse | Present, with a versioned entry. |
| Version field | Per entry (`"version"`), written by the process now writing, including after an upgrade; a downgrade resume wrote 2.1.250 entries. |
| Versionless record types | queue-operation, atis-latch, last-prompt, mode, permission-mode, file-history-snapshot, cost-state (skip when scanning). |
| Resume / continue | Append to the same file; SessionStart(resume) sees only the old file's newest version (so it must be buffered, not attributed). |
| Fork | Writes a new file; SessionStart has source `fork`. |

**Verdict:** attributable from the first tool event on; SessionStart (startup, fork, resume) is buffered by session id
and attributed on the session's first attributed event (as specified).

## Codex (0.159.3 → 0.160.0; read-only analysis of existing rollouts)

The live hook capture for Codex was not run: the permission layer refused `codex exec --dangerously-bypass-hook-trust`
and a `codex app-server` `hooks/list` probe (BLOCKED-AUTH). The rollout files answer the attribution question
without it:

| Question | Finding |
| --- | --- |
| Version field | `session_meta.payload.cli_version`, first line of the rollout. |
| Later `session_meta` records | 8 of the latest 400 rollouts have a second one, at line 2 with the same version (subagent/fork metadata, not resume). |
| Resume | A rollout created on 0.159.3 and resumed after the CLI updated to 0.160.0 kept a single `session_meta` (`0.159.3`) and appended turns with no version field. |

**Verdict:** a fresh Codex session is attributable from `session_meta.cli_version`. A resumed Codex session
(SessionStart source `resume`, or any event of a rollout whose session started before this process) cannot be
attributed from the rollout, because the recorded version is the creator's, not the resumer's; it records nothing
("codex resume: rollout version is the creating CLI's").

## Not established

- Codex SessionStart/tool-event timing relative to rollout creation under hooks (the refused capture).
- Codex shared app-server daemon vs `--no-daemon` differences for hooks.
