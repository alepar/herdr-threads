I1 ADDRESSED — supported canonical legacy same-event registrations now participate in conflict detection.
Spec CLEAN.
Quality CLEAN.

- `src/harness/setup.rs:272`–`296`: the new `legacy_command_route` recognizes the canonical installed legacy shared command and exact legacy evented command, optionally removing only the existing validated trailing UUID comment. It checks the registered event when present and reconstructs the entire canonical legacy command before accepting it. Arbitrary shell text and wrong event suffixes do not become accepted routes.
- `src/harness/setup.rs:334`: the existing shared preflight/status predicate now falls back to this legacy recognizer after stable recognition. The existing exact recorded-group exception remains unchanged, so recognition grants no ownership and legitimate recorded legacy migrations remain exempt.
- `tests/setup_cli.rs:4936`–`5049`: eight independent public CLI regressions cover Claude/Codex, shared/evented legacy commands and fresh/already-owned installations. They assert actionable setup refusal, truthful registration conflict, unchanged config and manifest bytes, and unsetup preserving the distinguishable foreign group plus ordinary before/after groups. This exercises the failing public path rather than only the helper.

New breakage in the fix diff: none.
Out-of-scope observations: none.

Checks:
- Inspected the complete 8700-byte fix package, its two-file stat and full test diff (117 added lines, below the 400-line cap). No existing assertions were removed or weakened, no skips/ignores or artificial historical version changes were added, and no extra production surface changed.
- Inspected the appended fix report: the eight public regressions failed before the production fix (0 passed, 8 failed) and passed afterward (8 passed); setup83 and CLI90 passed, totaling 173 unique tests. Reported all-target/all-feature clippy passed in 25.08s, format passed, and scoped leak scan reported no leaked test processes. The production change matches that covering test matrix.
- No Cargo, tests, native harnesses, Herdr/thread commands, git commands, subagents or source/index/branch changes were run during this rereview.

Fix round: All findings addressed, no new Critical/Important breakage.
