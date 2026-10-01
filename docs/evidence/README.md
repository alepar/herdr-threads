# Evidence kept in the tree

The full evidence of the herdr-threads development run (native demo, matrix and TUI runs, review records, logs and transcripts) is archived in git tag `archive/herdr-threads-run-2026-09-26` under `docs/superpowers/runs/2026-09-26-herdr-native-mailbox-thread-plugin/`. This folder keeps only what the code, the tests or the user docs read or cite, under the same folder names:

| Folder | Used by |
|---|---|
| [`claude-284-hook-capture/`](claude-284-hook-capture/report.md), [`claude-285-hook-capture/`](claude-285-hook-capture/report.md), [`claude-286-hook-capture/`](claude-286-hook-capture/report.md) | Evidence of the Claude recipe (`src/harness/claude.rs`), [integrations/claude](../../integrations/claude/README.md) |
| [`codex-158-hook-capture/`](codex-158-hook-capture/report.md) | Evidence of the Codex recipe (`src/harness/codex.rs`); its `schemas-0.158.0/` are read by the schema-fingerprint tests |
| [`codex-158-live-hook-capture/`](codex-158-live-hook-capture/report.md) | Evidence of the Codex recipe; source of `tests/fixtures/codex-0.158.0-live/` |
| [`codex-1593-sandbox-probe/`](codex-1593-sandbox-probe/README.md), [`codex-sandbox-writes-probe/`](codex-sandbox-writes-probe/README.md) | Codex sandbox allowance measurements ([install](../install.md), `src/cli/setup.rs`) |
| [`native-claude-demo-2/`](native-claude-demo-2/report.md), [`native-codex-demo-2/`](native-codex-demo-2/report.md) | Reports only, cited by the integration READMEs and the install guide |
| [`host-recovery-validation-tip/`](host-recovery-validation-tip/report.md), [`package-validation-tip/`](package-validation-tip/report.md) | Reports only, cited by the [validation report](../validation/report.md), [operations](../operations.md) and [release](../release.md) |

Files a kept report names but that are not here are in the archive tag at the same relative path.
