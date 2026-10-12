Spec ISSUES — one Important missing compatibility case in the same-event conflict contract.
Quality NEEDS FIXES — the shared conflict predicate silently excludes supported legacy registrations.

Strengths:
- `src/harness/setup.rs:1533`–`1715` separates exact manifest ownership from stable command identity, retains a durable prepared migration, and preserves exact owned groups in their existing slots. The UUID stays in the manifest.
- `src/harness/setup.rs:737`–`891` and `:1997` retain exact generation/cardinality checks during recovery and inspection. The changed tests cover ambiguous duplicates, foreign groups, declaration drift, real v1/v2 records and interruption boundaries.
- `tests/setup_cli.rs:4898` exercises the public setup/status/unsetup path and verifies native trust bytes and group indices remain unchanged. The report provides meaningful RED outcomes, 241 focused Rust GREEN results, offline consumer checks, clippy 43.00s, format and scoped cleanup. No Cargo was rerun in review.

Important I1 — recognize supported legacy registrations in the same-event conflict check.
- Evidence: `src/harness/setup.rs:304` calls `native_command_route`, whose `:248` requires `native_event_command`; that validator only accepts the dedicated `hook-event` shape. Therefore every valid historical `hook HARNESS --event EVENT` and shared `hook HARNESS` command is excluded, including canonical UUID-marked commands from another route. The fresh/reinstall preflights at `:1578` and `:1671`, and the diagnostic at `:3359`, all inherit this exclusion.
- Actual public-path reproduction, using the existing task-built `target/debug/herdr-threads` with an isolated temporary HOME, CODEX_HOME, CLAUDE_CONFIG_DIR, fake Codex executable, state directory and endpoint: seed `hooks.json` SessionStart with `'/x/herdr-threads' '--state-dir' '/foreign' 'hook' 'codex' '--event' 'SessionStart' # herdr-threads-owner:0b5e8f3c-1111-4222-8333-444455556666`; invoke `setup codex --state-dir <scratch>/state --host-endpoint <scratch>/host.sock --json`. Setup exits 0 with action `installed`, retaining the foreign legacy group and adding the stable SessionStart group. `setup-status codex` then reports `installed: true, registration_conflict: false`. Both registrations execute for the same event, despite the required safe refusal for conflicting same-event registrations.
- Fix: conservatively recognize the already supported canonical legacy evented/shared forms (including a real canonical trailing UUID marker) for conflict diagnostics, alongside stable forms. Keep this recognition separate from ownership: only the exact manifest-recorded groups may bypass the conflict during an owned migration; unrelated hooks stay preserved and distinguishable foreign groups remain safe to retain during exact unsetup. Add fresh and installed-manifest public-path regression cases for both native harnesses and legacy forms, with no config/manifest changes on refusal and truthful status. Preserve the existing valid owned legacy migration/adoption tests.

Minor: none.
Critical: none.

Review checks:
- Read the complete 173488-byte review package in bounded passes, including all 15 files. Inspected its test stat, the initial 400-line test diff, and the remaining test hunks; no new ignore/skip, assertion-only no-op, relaxed timing or fake lowered-version migration was found. Stable copied-profile tests now assert foreign preservation; historical ownership remains exercised by explicit consistent legacy groups/manifests rather than today's producer.
- Named focused outside-diff check: to determine whether the legacy command could be rejected elsewhere, inspected the complete `merge`/`compose_json` context at `src/harness/setup.rs:455`–`553`. It only matches the proposed command and therefore does not close I1. Read the cut-off `plan_resume` context around exact-present processing; no broad repository crawl or git commands.
- Ran only the isolated already-built public CLI reproduction above. No Cargo, real native harness, daemon, Herdr, real user config, source/index/branch mutation or messaging command ran. The temporary fixture was removed by TemporaryDirectory after both CLI children exited.
- Cross-task scope: final dependency composition, Task10 readiness policy, post-roast default-feature verification and whole-epic integration remain controller-owned; this review makes no claim about those later gates.

Task quality: Needs fixes.
