super-roast verdict: clean (0 nits, 3 FYI) [low coverage]
mode: PR        iteration: 1 of 3
profile (assumed): Internal developer tooling (local same-user CLI). The change touches only read-side classification of a symlinked Codex rules file; nothing is written through the link either before or after the change.
inputs: bead/ht-mf2q@cf1f45c4a vs main@103e9fbec
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
