# Moved approved runtime contracts — planned composition map

- src/cli/watch.rs::drain Lazy invitation classification -> src/delivery/assemble.rs::Assembler::push, preserving released generic page/drain architecture. Lazy remains pending and inbox/channel-discoverable, with no legacy-watch attention marker; this does not waive or hide canonical invitations. Existing tests/cli/watch.rs::invitation_policy_lazy_is_bookkeeping_while_timed_and_legacy_raise_attention will run against the moved producer before the scoped suppression change.
- src/harness/setup.rs::plan_claude stable per-event proposal -> neutral plan_settings(kind,...), preserving main SettingsKindSpec declaration.
- src/harness/setup.rs::plan_codex stable per-event launch configuration -> src/harness/codex/setup.rs::plan_codex; no restored neutral-core Codex branch.
- src/harness/setup.rs::valid_owner_marker historical v1 native route -> dynamic registered SettingsKind lookup, replacing removed fixed Claude/Codex kind variants without widening ownership recognition.
- src/cli/handoff.rs::HANDOFF_HELP -> src/cli/help.rs::HANDOFF_TEMPLATE generic body/policy/routing prose, retaining adapter fragments/kind/options assembly. Old setup-specific help remains adapter-owned main text.
- src/cli/journal.rs::record prepublication original bounds + main one-buffer durable write both retained.
