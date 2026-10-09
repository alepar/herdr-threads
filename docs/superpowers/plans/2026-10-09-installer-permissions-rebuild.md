# Installer permissions — rebuild on main

Supersedes the remaining open work on the `installer-permissions` branch (epic `ht-uwd`), which is kept
as reference only. Design intent is unchanged from
`docs/superpowers/runs/2026-10-07-installer-human-permissions/2026-10-07-installer-human-permissions-design.md`
(## Goal and Decisions 1–5) plus the configuration-backup amendment, with the adjustments below.

## Why a rebuild

`main` moved setup into a registry/adapter pipeline (`src/harness/{adapter,registry}.rs`, per-harness
`src/harness/{claude,codex}/setup.rs`) and added Hermes as a built-in harness. The branch's
orchestration lives in the old `cli::setup::execute` body that main replaced. The human grammar,
actor routing, retry actor preflight, ordinary catalog and `PermissionCliInputs` parsing already
landed on main (merge-base `3573b9f5`).

## Decisions (user-approved 2026-10-09)

- No legacy import: `.herdr-threads-claude-transfer.json` never shipped. No transfer journal at all.
- Hermes is in scope.
- Backups cover user configuration only: Claude `settings.json`; Codex `hooks.json`, `config.toml`,
  `rules/herdr-threads.rules`. Not SKILL.md or Hermes plugin assets (herdr-threads-owned, refused when
  edited), and never manifests/locks/state.
- Hermes enforcement is a `pre_tool_call` hook in the shipped Hermes plugin returning `approve` for
  `herdr-threads human …` terminal commands. No Hermes YAML edits; ordinary commands need no grant.
- `setup`, `unsetup`, `doctor fix` and `internal installer-integrations` leave the ordinary agent
  allow catalog: an agent must not be able to grant itself permissions.
- Lighter process: focused tests + `nice cargo clippy --locked --all-targets --all-features -- -D warnings`,
  `nice scripts/check-default-features`, `cargo fmt`, `git diff --check`; one independent review per
  phase; plain-language progress notes. Per-worktree `CARGO_TARGET_DIR` (a shared target dir hands
  back binaries built from another worktree). Main session owns landing, full suite and release.

## Phases

1. **Namespace rendering and frozen retry.** Port from the old branch: `protocol/output.rs`
   `CommandNamespaceGuard`/`namespace_argv` (prefix `human` on declared argv fields only),
   `cli/human.rs` invocation scope, `journal.rs` `pending_command_argv`, `store/queries.rs`
   human-caller detection, `store/seats.rs` refusal spellings, `me.rs` help; ht-uwd.18 frozen Human
   ordinary retry (`PendingIntent::ordinary_replay_claim`, `retry::preflight_ordinary_claim`,
   `mod.rs` replay selection). Tests: `human_guidance_*`, `frozen_origin_*`,
   `frozen_human_ordinary_retry_*`, operator UX spellings.
2. **Backup-and-publish helper.** One user-config writer in `harness/setup.rs`: existing file →
   `<name>.<UTC ts>-<uuid>.herdr-threads` byte copy (0600, create_new, synced) → same-dir temp →
   rename → parent fsync; preserves the active file's mode. Creation of an absent file is an
   exclusive publish with no backup; owned deletion backs up first. No-op writes create nothing.
   Route every non-private `write_replacement`, `OwnedFile::prepare` creation and `delete_created`
   user-config site through it. Reuse pieces of the old branch's `native_config.rs` only where they
   stay small.
3. **Permission core.** Transplant `harness/permissions.rs` (inputs/inventory/consent/plan/lock),
   `permissions/codex.rs` renderer, `permissions/claude.rs` renderer. Add
   `HarnessAdapter::permission_policy()` beside `installer_policy()`, forwarded through
   `ErasedAdapter`/`Registration`. Remove the self-grant commands from the catalog.
4. **Claude component.** Split the allow rule out of the hooks transaction (`DECLARED_RULE`,
   `OwnedPermission` in the hook manifest stay readable for historical state). Migration of the
   historical broad rule: permission manifest records pending intent (settings fingerprints
   before/after) → one atomic settings.json write adds the narrow rules and removes the broad rule →
   hook manifest drops its permission record → pending intent cleared. Recovery: settings match
   before or after ⇒ finish the remaining steps; anything else ⇒ refuse with guidance.
   Pre-existing/foreign rules untouched; historical ownership without consent only narrows.
5. **Codex and Hermes.** Codex `CODEX_HOME/rules/herdr-threads.rules`: allow prefix rule for the
   spelling union, prompt rule for `… human`; own manifest; backed writes. Hermes: plugin
   `register(ctx)` adds the `pre_tool_call` approve hook; status reports it as part of the plugin.
6. **Wiring.** Consent flags (`--permissions`, `--with-permissions`, `--without-permissions`,
   inventory paths) through setup/status/unsetup/bare setup, `internal installer-integrations`
   (permissions component + prompt), `install.sh` (`--without-permissions`, capability probe,
   uninstall), doctor (report; fix only narrows). Behaviour spec and tests ported from the old
   branch's `setup_cli.rs`, `installer_integrations.rs`, `tests/cli/installer.rs`,
   `tests/release/install_test.sh`.
7. **Docs and review.** install/integration guides, TRUST-POLICY.md if an invariant changes; whole
   branch review, one roast, fix loop.
