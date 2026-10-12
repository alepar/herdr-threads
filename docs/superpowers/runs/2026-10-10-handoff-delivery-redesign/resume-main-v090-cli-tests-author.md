# Main v0.9.0 CLI test composition author report

Status: QUIESCENT. Working files resolved; parent owns index, merge commit, Cargo and runtime validation.

Checkout: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/upstream-composition-v090`.
Observed HEAD: `f330bdab3abee8535213228e5465b085b65be750`.
Merging Main: `b6dd5dd122772599a02e2cc7f76b20e5492f0af3`.

## Scope and decisions

- Changed only `tests/cli/commands.rs`, `tests/cli/cooperative.rs`, `tests/harness/setup.rs`, and `tests/setup_cli.rs` in the checkout. This report is the sole additional write. No staging, commit, cherry-pick, Cargo, build, tests, daemon, or helper processes.
- Read stage-3 Main TRUST-POLICY before attribution work. Preserved Main session/binding claim fields, subagent read-only cursor behavior, and the cooperative trust model. No production changes.
- Inspected stages 1/2/3. Combined independent complete test groups instead of choosing either conflict side. Commands retain three handoff invitation policy cases plus three Main agent-id cases. Cooperative retains two handoff intent/capability cases plus two Main subagent cursor cases, with all earlier auto-merged tests unchanged semantically.
- Setup unit tests assert exact Main event sets: Claude SessionStart/PreToolUse/SubagentStart/SubagentStop; Codex SessionStart/SubagentStart/PreToolUse/SubagentStop/SessionEnd. Every declared event must have the stable hook-event command. Deliberate historical owner-marker fixture/helper cases remain intact.
- setup_cli retains stable reinstall and all eight legacy foreign-registration cases together with Main lifecycle/stale-install cases. Converted newly introduced Main installed-command assertions and ownership-count helper to stable hook-event syntax. Codex stable reinstall compares all five CODEX_OWNED_EVENTS. Exact unsetup original-byte oracles remain.
- Main stale-install fixtures start with this build's actual setup output then remove named events from recorded ownership and config. No package-version literal or 0.8.1 assumption added; current Cargo metadata is 0.9.0. Stale-install missing-event/fix assertions are preserved.

## Static verification

- `rustfmt --edition 2024 --check` for the four owned files: exit 0.
- `git diff --check -- <four owned paths>`: exit 0.
- No conflict markers remain in these four paths.
- Test case-name comparison against both stages: every test added by either side retained; no duplicate test names. Resolved test counts: commands 95, cooperative 75, harness/setup 87, setup_cli 101.
- File-specific rustfmt expanded pre-existing dense commands.rs formatting (stage-2 2,601 lines; resolved 3,854). This is formatting noise within the owned path, not added semantic test scope.

## Remaining validation boundary

- No compiler/runtime outcome claimed. Parent must type-check and run composed tests after all merge owners finish.
- Preserved main539_composition fixture still seeds a legacy context-backed agent claim and canonical occupant row. Main itself left that fixture intact; ContextJournal explicitly supports its non-agent saved-context handle. New Main subagent cases use with_agent_caller plus TEST liveness. No broad fixture migration made; canonical claim admission against the newly composed store remains a runtime check for the parent.

## Working-file fingerprints

- `tests/cli/commands.rs` SHA256 `e2dcac6ae30e0072b3af6337553c717633088f8e3dc154b5c27ffe2decbf8e35`

## Input stage blobs

- `tests/cli/cooperative.rs` SHA256 `8dda5326504c08fb0d59ca2b695f88a01e27b847502124c8dfe7787d0ac567d9`

## Input stage blobs

- `tests/harness/setup.rs` SHA256 `25e26be59c381a8d994bac8eac384db367be5be67b98f5e61889aefe8987bfbb`

## Input stage blobs

- `tests/setup_cli.rs` SHA256 `4f5c1aeca7cfc6e74ecde2fdb152ec1240bbfd27ebeeb474f962639a09bc970f`

## Input stage blobs

- `tests/cli/commands.rs`: base `683f1d131fcaf725f3ce309e1d4081c61a16f29f`, ours `b5f9e05e44871c8340bfa4677ce2c6cf6a6a7975`, Main `f078a030b30e1d663b4b44311c3a7708c8c1e5c5`.
- `tests/cli/cooperative.rs`: base `d2b5b1b700cfdd3763502c85e5083398c757ad0e`, ours `59081110381c3dc03080d2b8e0fc60525c9d0406`, Main `6ae146d4846853b301de4c6b30444e688ed12d6a`.
- `tests/harness/setup.rs`: base `a3ef66443c2fe5f37fbebc824704473ebb7b17f2`, ours `2f169fcf6427b04a9303e9c2c03818adf02291cd`, Main `2ad68784fe6c0904803974a7968f62c62cef6b55`.
- `tests/setup_cli.rs`: base `20ec555c7ceabc9e49a0365a597babe4dc11e03e`, ours `8556e51b5b61f0d03c67ee163fa92159eaefb427`, Main `25d89d5319680d1b61b4695d0ab5955d46aa7dd3`.
