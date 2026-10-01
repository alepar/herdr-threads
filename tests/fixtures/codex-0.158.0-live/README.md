# Codex 0.158.0 live hook-input fixtures

Byte-identical copies of
`docs/evidence/codex-158-live-hook-capture/payloads/`.
These are **live captures** from `codex-cli 0.158.0` (`codex exec`, `-s read-only`,
hooks passed only as `-c` overrides; see that directory's `report.md`). Each hook
file wraps the native stdin payload as `payload`, the hook's stdout as
`hook_stdout`, and the Herdr/Codex env var names the hook saw. Paths were redacted
at capture time to the placeholders `$S` (scratch dir) and `$CODEX_HOME`.

| File | Event | Hook stdout |
| --- | --- | --- |
| `run1.session-start.startup.json` | SessionStart `startup` (ephemeral) | none (silent logger) |
| `run1.subagent-start.json` | SubagentStart | none |
| `run1.pre-tool-use.root.json` | root Bash PreToolUse | none |
| `run1.pre-tool-use.child.json` | child Bash PreToolUse (`agent_id`/`agent_type`) | none |
| `run2.session-start.startup.json` | SessionStart `startup` (persisted) | context-only `additionalContext` |
| `run3.session-start.resume.json` | SessionStart `resume` (same session as run2) | context-only `additionalContext` |
| `run3.pre-tool-use.root.json` | root Bash PreToolUse (resumed session) | context-only `additionalContext` |

`run2-run3.rollout-model-items.json` is the model-visible excerpt of the persisted
run2/run3 rollout: each `additionalContext` marker appears as a `developer`
message (SessionStart before the user prompt, re-injected on resume; PreToolUse
after the tool call and before its output).

Evidence scope: context-only `additionalContext` delivery is proven for
SessionStart and PreToolUse only. SubagentStart output delivery was not
exercised. Invocation transport (`updatedInput` rewrite) and model receipt stay
Unsupported. `permission_mode` was `bypassPermissions` in every payload even
under `-s read-only`: it reflects the exec approval policy, never the sandbox,
and the adapter does not read it. No SessionStart `fork` payload was captured.
