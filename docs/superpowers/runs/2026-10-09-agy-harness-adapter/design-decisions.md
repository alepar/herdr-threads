# agy harness adapter — design decisions (root brainstorm, Mode A, 2026-10-09)

Working notes from the interactive brainstorm; the spec supersedes this file once written.

1. Enrollment: global hook enrollment like Claude/Codex (not launch-only like Hermes). Bare `agy` in a Herdr pane enrolls on first prompt.
2. Child vs /clear: a SessionStart is top-level only if `presence/<id>.lock` exists AND the pane's bound conversation does not list `<id>` under `brain/<bound>/.system_generated/subagents/`. Otherwise child: no seat change, child restriction answered locally. Hook gathers facts as evidence; daemon decides (A2). A missed /clear rebinds at the next turn-start PreInvocation re-check. TRUST-POLICY gains the on-disk witness rule.
3. Shell permission: setup owns one `command(herdr-threads)` entry in `~/.gemini/antigravity-cli/settings.json` `permissions.allow` via the shared JSON engine (owned array member, manifest, pre-existing kept). Status/doctor report a missing entry (agy rewrites this file). `--sandbox` socket reachability: documented/doctor-informational limit, no launch refusal.
4. Install shape: owned plugin directory `~/.gemini/config/plugins/herdr-threads/` (plugin.json + hooks.json) written by setup as owned assets (not `agy plugin install`); no `rules/` file; status reports disabled state from `config/config.json`.
5. Delivery: reuse v0.5.0 Claude-mod daemon delivery (D4 framing, paging, ack-by-id, routing suppression) with the agy hook acting as a pull-based mod: PreInvocation (every invocation) checks attention version, injects framed full bodies (injectSteps toolCall if the 1.3.2 probe passes, else userMessage). Idle: existing native wake; the woken turn's PreInvocation delivers. Settlement is transcript-confirmed: the next hook call verifies the injected ids in `transcriptPath` before acking, under a new sibling provenance (e.g. `cooperative_hook_delivery`, TRUST-POLICY). Lazy rows ride along. Expect generalization of Claude-specific v0.5.0 pieces (watch --harness claude, not_claude, harness='claude' checks) — record for the adapter-model report.

Side work: Hermes global enrollment split to bead ht-4ne, tab `hermes-global`.
