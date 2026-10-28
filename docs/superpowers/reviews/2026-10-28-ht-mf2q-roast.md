super-roast verdict: clean (0 nits, 3 FYI) [low coverage]
mode: PR        iteration: 1 of 3
profile (assumed): Internal developer tooling (local same-user CLI). The change touches only read-side classification of a symlinked Codex rules file; nothing is written through the link either before or after the change.
inputs: bead/ht-mf2q@cf1f45c4a vs main@103e9fbec (round 1, inline); independent review round below
coverage: unavailable: manual run, no coverage object. Lanes covered inline: correctness, regression vs existing tests, test causality.
independence: none (inline) · rung: inline

## Confirmed findings

None.

## Rejected (with reason)

- `shared_rules` changes the `managed_at` value for a profile from a record's lexical path to the canonical file, which could break consumers. Rejected: the only consumers are the installer's "managed at" text and `tests/cli/installer.rs:1521`, which already asserts the canonical path. The release script (`tests/release/install_test.sh:918`) checks only the prefix.
- Dropping the "real file absent while a record is Pending" case into Missing. Rejected: a Pending record sets `unsettled` whatever the file's state, so it still reports Unowned, as before.
- An Edited record whose file is present now falls through to the presence check rather than returning Unowned directly. Rejected: same outcome (present means Unowned). A Missing record implies an absent file, which gives Missing, as before.

## Unverified nits (spot-checked)

None.

## FYI

- [FYI] src/harness/codex/setup.rs `shared_rules`: an Installed record now wins over a Pending one from another root, whatever the order. Before the change, directory order decided this. It matches the ht-lmjf skill rule. Nothing is written through the link, and the real root's next setup settles the pending record.
- [FYI, preExisting] src/harness/codex/setup.rs `managed_elsewhere`: with several legacy records made through links, `.find` returns the first valid one in read_dir order, and the verdict names that record's link path (`tests/cli/installer.rs:1532` intends the link path). Only the reported path depends on order, not the outcome. Possible follow-up.
- [FYI] tests/installer_integrations.rs `installer_reports_recorded_profile_rules_shared_once_rules_is_a_symlink`: it reproduces the main-branch failure deterministically only on a newest-first directory listing (tmpfs, the default temp dir here). On a hash-ordered filesystem it can pass on main by chance. After the fix it holds on any order.

## Escalations (need human)

None.

## Independent review round (coordinator's reviewer), resolved in 8f809d5c0

- [Should-fix, confirmed, resolved] src/harness/codex/setup.rs `shared_rules` (and the same rule in src/cli/installer_skill.rs `inspect_shared`, on main via ht-lmjf): the first installed record won, so the real root's own edited or interrupted record had no veto. A profile reported the file shared and managed while the real root reported it edited. Fix: one neutral decision, `harness::setup::shared_ownership`. A record made at the root that holds the file (`shared_through_symlink(record_root, file) == None`, which also holds when the root itself is a link) decides. Only when there is no such record, or it is missing, does any installed record own the file. Test: `installer_lets_the_real_roots_edited_record_veto_a_linked_profiles_record` (rules file and skill) fails on cf1f45c4a.
- [Nit, resolved] The stale-record integration test was deterministic only on a newest-first listing. `shared_ownership_tests::the_real_roots_record_decides_in_every_order` now runs the decision over every permutation of each record set. It fails under both earlier rules ("the first record listed decides" and "any installed record wins").
- [FYI] If two roots both reach the real file without a link below them (for example CODEX_HOME=~/.codex, a link, and CODEX_HOME=~/dotfiles/codex, both recorded), any installed record among them wins over an unsettled one among them. The result does not depend on order, but it is not a strict veto.
- [FYI] The record's root is derived from the recorded path (`rules/herdr-threads.rules`, two levels up; `skills/herdr-threads/SKILL.md`, three levels up). Both layouts are fixed by their adapters, and the permission manifest's own `config_root` is written from the same derivation.
