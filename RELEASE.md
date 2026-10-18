# Releasing herdr-threads

How a release is cut, from the fixes to the installed, announced version. The releasing agent (the release coordinator) follows this file. Evidence for each published release lives in [docs/release.md](docs/release.md) ("Current release status"); background on the workflows, archives and Herdr matrix is in the same file and in [docs/install.md](docs/install.md).

## Rules

- **Version:** breaking change = major, new feature = minor, fixes only = patch. The release takes the highest level among its changes. A change to the node store schema is called out in the changelog (it is a one-way upgrade for older binaries).
- **Push policy:** push only `main` and the explicit release tag. Never `--tags` or `--mirror`, never other branches, never `git stash`. The tagger email is the GitHub no-reply address of the maintainer.
- **Commit trailers:** `Outside-adapter: <paths|none>` and the `Co-Authored-By` line the maintainer named (Claude Opus 5.5) on every commit.
- **Real user config is never written:** `~/.claude`, `~/.codex`, `~/.aisw`, `~/.gemini` (tests use isolated homes).
- **Shared Herdr server:** never restart, stop or kill it. Stopping or ensuring the herdr-threads daemon is fine.
- **No duplicate testing:** reuse the owners' evidence for the exact SHA; the merged candidate gets one full suite. A later commit that changes only tests or fixtures reuses the earlier suite, matrix and wake evidence if the production code is identical (say so in the record).
- **Flaky or over-budget tests do not block a release.** Hand them to a fixer tab and proceed. Only a legitimately failing test blocks.
- **The wake e2e runs before every release** (AGENTS.md, "Pre-release wake e2e"). A failure blocks until fixed or accepted by the user and recorded.
- **Never publish credentials** in evidence, records or announcements.
- **Channels:** release announcements go to the `herdr-threads-dev` thread. `psa-global` is for system-wide notices only.

## 1. Collect the fixes

Each fix is done by a fixer in its own worktree and branch, with a bead (`bd`), a regression test that failed before the fix, and a reported commit SHA and test evidence. Requests to fixers go out with `--nudge` (priority 0). Fixers never push, tag or release.

## 2. Release branch

In the standing release worktree (`.worktrees/herdr-095`): create `release/X.Y.Z` from `main`, cherry-pick or merge the fixers' commits, then add one "Prepare vX.Y.Z" commit: bump `Cargo.toml` and `herdr-plugin.toml` (line 3), run `cargo update -p herdr-threads --offline`, and add the `CHANGELOG.md` entry (heading `## vX.Y.Z`). On macOS use `sed -i ''`; check the diff, since a failed `sed` leaves the version unbumped.

## 3. Qualify the candidate

Run on the release worktree. Order matters only for the suite, which is timing-sensitive and runs alone:

1. `cargo fmt --check`, `scripts/check-harness-neutral`, `nice cargo clippy --locked --all-targets --all-features -- -D warnings`, `nice scripts/check-default-features`.
2. The full suite with a run id: `export HT_LEAK_RUN_ID=$(uuidgen)`, `nice cargo nextest run --locked --all-targets --all-features`, then `scripts/check-no-leaked-processes --run-id "$HT_LEAK_RUN_ID"`.
3. The Python installer suites CI runs (the exact commands are in `.github/workflows/ci.yml`: `tests/release/installer_alias_test.py`, `installer_integrations_test.py`, `installer_terminal_test.py`) and the shell installer harness.
4. `scripts/herdr-release-smoke` (the Herdr release matrix) and, from a Herdr pane, `nice scripts/build.sh`, `python3 scripts/wake-e2e.py --self-test`, then `scripts/wake-e2e-herdr <newest herdr in tests/herdr-releases.tsv> --evidence-dir DIR`. The matrix and the wake e2e use private Herdr servers and may run side by side after the suite.
5. Check that no scratch roots (`/tmp/ht-wake-e2e.*`) remain.

A real failure goes back to the fixer who owns the change (merge the release branch into their branch, fix, report a SHA), is cherry-picked, and the affected checks re-run. Keep the commands and results for the record.

## 4. Merge and CI

`git merge --ff-only release/X.Y.Z` into `main`, `git push origin main`. The push starts CI. Watch each job as it finishes and act on a failure at once; do not wait for the whole run. Failed job logs: `gh api --allow-escape-sequences repos/alepar/herdr-threads/actions/jobs/JOB_ID/logs`, with ANSI codes stripped. CI runs on Linux; the local run is macOS, so CI can find what the local run cannot (it found a stale installer fixture in v2.2.0).

## 5. Tag and publish

When CI is green on the release SHA: annotated tag `vX.Y.Z` (`git -c user.email=… tag -a`), `git push origin vX.Y.Z`. `release.yml` builds the four archives and publishes. Wait for it to succeed.

## 6. Verify the archives

For each of the four archives: `SHA256SUMS` equals the digests GitHub reports, `VERSION` and `TARGET` are right, modes are correct, the licenses are present, and a credential scan of the extracted tree is clean. Extract each archive into its own empty directory; never run anything from inside it.

## 7. Install and verify

1. **Back up first** on each host: a `sqlite3 .backup` copy of the store plus `node.key` and `node.json` (directory mode 700, files 600), under `~/.local/state/herdr/plugins/herdr-threads-pre-vX.Y.Z-backup-*`.
2. **This Mac:** `scripts/install.sh --version vX.Y.Z`, then `herdr-threads daemon stop; herdr-threads daemon ensure`, and compare the binary hash with the archive's.
3. **blacky.dayton:** python `sqlite3` backup, then `curl …/vX.Y.Z/scripts/install.sh | bash -s -- --setup --version vX.Y.Z`, `daemon stop` and `daemon ensure`, then check `/proc/PID/exe` against the archive.
4. Confirm both daemons report the new version and the peer link is active.

## 8. Record and announce

Add the release record to `docs/release.md` under "Current release status": source SHA, what changed, qualification results (commands and counts, plus what was reused and from which SHA), CI run links, archive verification outcome, install hashes. Commit and push `main`. Announce on `herdr-threads-dev` (version, headline changes, any one-way upgrade, what to do). Tell any tab waiting on the release.

## 9. Clean up

For each merged fixer: close its tab by its verified label, `git worktree remove` its worktree, `git branch -d` its branch (a fixer branch whose commits were cherry-picked needs `-D`; ask first), and close its bead with `bd close`. Use a subagent for the cleanup. Keep `main` and the release worktree.

## Open improvements

Release lead time is being cut (bead ht-z70k): CI starts alongside local qualification, the manual CI dispatch is dropped, the local steps run through one script, docs-only commits skip heavy CI, the release workflow reuses CI builds, and CI cache time is investigated. This file changes with each of those.
