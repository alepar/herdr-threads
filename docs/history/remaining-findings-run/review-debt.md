# B8 review debt (ht-p03.19)

Run: remaining herdr-threads findings, integration branch `super-auto/remaining-herdr-threads-findings`, reviewed at tip `eb5c59da`.
Epic base: `55512edb` (merge-base of `main` and the integration branch).
Inputs: archive tag `archive/herdr-threads-run-2026-09-26` (read with `git show`, never checked out), `docs/history/herdr-threads-run/`, `docs/history/remaining-findings-2026-10-01.md` section B8.

## 1. Exclusion set

Command (run in the task worktree, which starts at the integration tip):

```
git diff --name-only 55512edb HEAD -- src scripts tests .github Cargo.toml | sort > excl.txt
git ls-files src scripts | sort > all.txt
comm -13 excl.txt all.txt        # source and script files NOT rewritten by this run
```

The bead's suggested `git log --grep` form does not fit this run's commit convention: task work is merged with `Merge branch 'task-ht-p03.N'` commits and only the B5 beads are absent from the epic. Every bead on the integration branch since the base except B5-frozen code and the review leaf itself is one of the B1-B4/B6/B7/B9/B10 beads or a seam-integration fix, so the union of changed paths is the exclusion set and is a superset of the merged diff of the beads the bead names (B9 test-only changes are included, which only widens the exclusion).

Size: 97 files under `src/`, 3 under `.github/workflows`, `Cargo.toml`, and about 50 files under `scripts/` (canary, validator corpus, installer) changed since the base. Of 7 412 lines in `src/` and `scripts/` files, only the following files were not rewritten and form the reviewable remainder:

| lane | not-rewritten files (lines) |
|---|---|
| short IDs | `src/store/public_ids.rs` (124), `src/protocol/ids.rs` (215) |
| read paging | `src/protocol/pagination.rs` (941), `src/store/operator.rs` (131) |
| retry journal (regression-pass fix) | `src/cli/retry.rs` (339) |
| release/installer | `scripts/package-release.sh` (71), `scripts/build.sh` (26), `scripts/view.sh` (35), `scripts/validate-package.sh` (645), `scripts/validate-host-recovery.sh` (25), `scripts/demo-tea-party.sh` (308) |
| daemon/host | `src/daemon/ownership.rs` (667), `src/daemon/diagnostics.rs` (187), `src/host/continuity.rs` (197) |
| other | `src/protocol/attention.rs` (232), `src/protocol/service.rs` (580), `src/store/service_controls.rs` (728), `src/store/invitation_due.rs` (201), `src/service/config.rs` (218), `src/scheduler/config.rs` (72), `scripts/reconcile-validation*.py`, `scripts/native-fixture.py` |

Everything else in the six light-review lanes (`src/harness/setup.rs`, `src/cli/setup.rs`, `src/cli/human.rs`, `src/cli/follow.rs`, `src/cli/output.rs`, `src/cli/hook.rs`, `scripts/install.sh`, `.github/workflows/*`) was rewritten by this run and is reviewed by this run's own per-task and final code review. Those lanes are therefore recorded as covered, not re-read here.

## 2. Findings

Review depth: files in the lane remainder above were read (retry.rs, public_ids.rs, ids.rs, ownership.rs `reclaim_dead_socket`, operator.rs, package-release.sh, build.sh, view.sh, the head of validate-package.sh) or machine-checked (`sh -n` on every shell script, `shellcheck -s sh`, `python3 -m py_compile` on the Python scripts). pagination.rs, service_controls.rs, protocol/service.rs, invitation_due.rs and demo-tea-party.sh were not read line by line; they are not named by any bead, parked item or concern, and they are listed as unreviewed in section 7.

| id | severity | location | finding | disposition |
|---|---|---|---|---|
| RD-1 | Minor | `src/cli/retry.rs` `is_deterministic_rejection`, `tests/harness/bridge.rs:1312` | The cooperative discard list omits codes that are equally request-determined (`NotFound`, `Unsupported`, `UnsupportedHarness`, `ThreadNotOrphaned`, `RequiredInvitationNeedsManagedThread`, `StaleRequirementAcceptance`), so a refused cooperative `send` to a mistyped thread id leaves a stale `pending-ops` entry. The test covers `InvalidRequest` plus the transient codes only, so `Unauthorized`, `Archived`, `Conflict`, `OperationPayloadMismatch` and `MembershipRequired` have no cooperative-scope test. Keep-on-doubt is the documented rule, so nothing is lost or duplicated; the cost is stale entries. Origin: regression-pass fix ht-4is.37. | filed ht-p03.121 |
| RD-2 | Nit | `src/protocol/ids.rs` `public_id_suffix` | Doc claims 104 random bits and bias below 2^-56. The loop keeps 14 bytes (112 bits), so the code is correct and the bias is smaller; the comment is wrong. `src/store/operator.rs` replay comment also has missing spaces. | filed ht-p03.122 |
| RD-3 | Nit | `scripts/build.sh:4`, `view.sh:6`, `validate-host-recovery.sh:17-18`, `validate-package.sh:18-19` | shellcheck SC1007 on `CDPATH= cd`. Intended behavior; `package-release.sh` already writes `CDPATH=''`. Only matters when a shellcheck CI step is added. | filed ht-p03.123 |
| RD-4 | none | `src/daemon/ownership.rs` `reclaim_dead_socket` | Re-checked parked P38 (macOS full-backlog ECONNREFUSED): the doc comment now states that the exclusive owner lock proves no live owner and that only a same-uid socket refusing connections is removed, with an inode re-check before unlink. No action. | no finding |
| RD-5 | none | `scripts/package-release.sh`, `scripts/build.sh` | Read for argument handling, staging cleanup (`trap` on EXIT/HUP/INT/TERM), atomic archive `mv`, PREBUILT contract, pinned `CARGO_TARGET_DIR`. `set -u` makes a flag with no value abort rather than mis-parse. No defect found. | no finding |
| RD-6 | none | `src/store/public_ids.rs` | Collision check runs inside the caller's write transaction against every derived slot; 16 attempts then `StoreCorrupt`; the table UNIQUE constraints are the backstop; three unit tests exercise regeneration, exhaustion and derived slots. Table names in the `format!` SQL are `&'static str` constants. No defect found. | no finding |

No Should-fix or Blocker finding was found in the reviewable remainder, so no source fix and no blocker bead was needed in this leaf. The full serial suite was not re-run: no source or test file changed.

## 3. Regression-pass fixes never re-roasted

Round-2 regression pass (`run.md:472-474`) filed two Should-fix beads and skipped the re-roast.

| fix | commits | files | status at tip |
|---|---|---|---|
| ht-4is.37 discard intents only on deterministic rejections | `de0c4e0e`, `ad905554` (scoped to cooperative intents) | `src/cli/retry.rs`, `tests/harness/bridge.rs` | `retry.rs` is outside the exclusion set and was read in full: RD-1. |
| ht-4is.38 shared resolved-first pane seat selection | `2be3b98c` | `src/cli/hook.rs`, `src/cli/mod.rs`, `tests/cli/hook.rs` | All three files were rewritten by B6/B10/B3 beads in this run, so they are in the exclusion set and covered by this run's own review. |

## 4. The 17 DONE_WITH_CONCERNS reports

The reports lived under `.superpowers/sdd/ht-4is-plan/`, which is git-ignored and was never committed. They are not in the archive tag and not on disk in this repository or its worktrees (searched the repository root and the plan workspace). They cannot be re-read one by one. Recorded disposition: accepted with reason.

- The final report (`docs/history/herdr-threads-run/report.md:204`) characterizes them: "Most of the concerns were provisional bases or open native gates that later closed." The concerns were against the 2026-09-26 tree; the 100-row parked-disposition table (`parked-disposition.md`) already re-checked every code-verifiable concern from the 09-30 reviews against `0853d62` and sorted them FIXED, FIX-NOW, ROAST or WAIVE.
- Tip evidence for the FIX-NOW rows that came out of those reviews, verified here by grep at `eb5c59da`: P4/W10-Doc1 (Codex hook trust documented: `docs/install.md:189`), W6-C1 (`--remote`, `--remote-auth-token-env`, `--thread-source` in `CODEX_VALUE_OPTIONS`, `src/harness/launch.rs:127-129`), W6-C2 (`key == "hooks" || key.starts_with("hooks.")`, `launch.rs:426`), W6-D2 (`service disconnect` in the validator mutating verbs, `scripts/validate-native-demo.py:156`), W9-4 (`cooperative_wake_refuses_when_incarnation_or_epoch_moves_during_recheck`, `src/host/native.rs:2192`).
- The remaining open items are the B8 validator beads (ht-p03.18, closed), the native matrix rerun (ht-p03.20) and the closure ledger (ht-p03.37), which carry the live-evidence part.
- The report names, among the 17, the cooperative-harness reports, comp-host-seats, claude-sessionstart-setup and host-recovery-validation; their native-evidence part is what the ht-p03.20 matrix rerun re-measures. This is an inference from the report's wording, not a check of the lost texts.

Residual risk: a concern that was neither code-verifiable at `0853d62` nor an evidence gap is unrecoverable. Accepted; flagged for the owner in section 6.

## 5. Parked items P41-P44

`parked-findings.md` (backfill, commit `e2288e8`) marks all four "detail not enumerated": the originating reviews were git-ignored. Counts survive in `run.md`. With the text gone, each was triaged by the code it touched, which can still be computed from the merge commit. A path in the exclusion set is covered by this run's own review; a path outside it was reviewed here.

| id | origin | parked content | files touched | disposition |
|---|---|---|---|---|
| P41 | package lifecycle 10.3 fix3 `1b93360`, 0 blocking / 1 important-or-minor / 2 nits | not recoverable | `scripts/validate-package.sh` only | Reviewed here: syntax and shellcheck clean apart from RD-3 (SC1007, filed ht-p03.123); the isolation properties the lifecycle depends on (private HOME/XDG/socket, pinned Herdr sha256, shared-config snapshot compare, hostile inherited `CARGO_TARGET_DIR`) are present at lines 18-140. Remaining parked content: explicit accept, reason: unenumerable, and the script's own acceptance (`PACKAGE_VALIDATION_PASS`, 89 checks at `d635aca`) is the gate to rerun on the tip (not run by this leaf). |
| P42 | flaky-probe 11.7 `e606b34`, 0/0/1 | not recoverable | `src/protocol/time.rs`, `src/store/{mod,queries,service_events}.rs`, `tests/service*.rs`, `tests/store/control.rs` | Every file was rewritten by B1/B3 beads in this run: covered by this run's review. Explicit accept for the unrecoverable nit. |
| P43 | hook+digest semantic merge `08612f8`, seam review CLEAN 0/0/3 nits | not recoverable | 27 files, all rewritten in this run except `src/protocol/attention.rs` | `attention.rs` read as part of P21 (`deny_unknown_fields` mixed-version window), already WAIVED as disclosed single-binary. Remaining files covered by this run's review. Explicit accept. |
| P44 | Claude 2.1.285 recipe merge `05a662c`, 0 / 2 minor / 3 nits | not recoverable | `src/harness/claude.rs`, `tests/harness/{claude,recipe}.rs`, `tests/lifecycle_ux.rs` (all rewritten by B6), plus capture fixtures `tests/fixtures/claude-2.1.285/*.json` | Code covered by the B6 recipe/admission rewrite (ht-p03.13) and its review; the capture fixtures are data. The recipe table has since moved to the harness version source of truth, so 2.1.285 now lives in `docs/compatibility/harness-versions.json` with an evidence level. Explicit accept. |

No P41-P44 item was converted to a bucket fix: nothing in them can be named without the lost review text, and filing an unnamed issue would not be actionable.

## 6. P45 goal-vs-tree table (flagged for the owner's read at finish)

Goals are from the root design `2026-09-27-herdr-threads-design.md` (Goal, Product boundaries, Seat and occupant, Prelaunch flow, Membership, Timing). "Tree" is the integration tip. The mandatory human goal-vs-full-tree read-through was parked by the first run ("parked under autonomous continuation, not treated as completed"); this table is an agent-written substitute and is not a read-through. The owner should read it, and decide whether a human read of the live demo is still required before release.

| goal | where the tree meets it | gap or caveat |
|---|---|---|
| Real Herdr plugin, installable from `alepar/herdr-threads` with a valid manifest | `herdr-plugin.toml`, `scripts/{view,build}.sh`, `scripts/package-release.sh`, `scripts/validate-package.sh`, `.github/workflows/release.yml`, v0.1.0 notes in `CHANGELOG.md` and `docs/release.md` | Clean-install lifecycle evidence is from the pre-run SHA (`d635aca`, 89 checks); rerunning `validate-package.sh` on the tip is not done by this leaf. No release is cut from this run (hard constraint). |
| Persistent, discoverable group threads with explicit topic | `src/store/{messages,queries,seats,control}.rs`, `src/cli/commands.rs` thread verbs | Covered by unit and integration tests; live both-harness gate ht-910 still unresolved (see below). |
| Seat = role bound to a pane; occupants replaceable; host mappings are lookup only | `src/identity/*`, `src/store/seats.rs`, `src/host/{native,continuity,observation}.rs` | Adapter never reports `EmptyShell` (P10): an occupant that exits while the pane stays open is unseated only by the next lifecycle hook, pane close or incarnation change. Accepted gap since the first run. |
| Pane closure retires seat; label reuse does not resurrect; host-down is not closure | `src/store/{seats,materialization}.rs`, `src/service/*` retirement jobs | Retention lane (B1) now prunes snapshots and work jobs; retirement cleanup lag is exposed. No new gap found. |
| Invitations and messages before an agent exists; prelaunch flow | `src/store/{messages,invitation_due}.rs`, `src/cli/launch.rs`, `src/harness/*` | Live prelaunch demo evidence predates this run's code. |
| ACK means explicit receipt only; batch idempotent | `src/store/receipts.rs`, `src/cli/commands.rs` | Covered by tests. |
| Native top-level caller proof for accept/ACK (superseded to cooperative by user direction 2026-09-28) | `src/harness/{bridge,context,claude,codex}.rs`, `src/cli/hook.rs`, `src/store/attention.rs` | Direction is cooperative; the old native-proof clauses are historical. Native TOCTOU guards in `src/host/native.rs` remain with tests. |
| Operator mode: local-user rebind, fresh seat, orphan invite only | `src/store/operator.rs`, `src/store/service_controls.rs` | Reviewed RD-6 neighbors; no gap found. |
| Deadlines and warnings: durable, unique, no hard failure | `src/scheduler/deadlines.rs`, `src/store/{wake,work,invitation_due}.rs`, Pacer lanes (B2) | W9-1 (wake ladder climbing on pre-send refusals) addressed by ht-p03.9.3. |
| Context-efficient, inspectable output | B10 presentation contract (ht-p03.17), shared escaping (ht-p03.33), human output, `src/protocol/output_compact.rs`, `src/cli/{human,irc,follow}.rs` | Golden tests under `tests/cli/golden`; token-overhead benchmark never produced (B8 "Token overhead" Gap, not covered by any bead named in this leaf). |
| Fail-closed on version skew and unsupported harness | B3 taxonomy (ht-p03.10), B6 admission ladder (ht-p03.13), canary (ht-p03.14.x) | Canary tier 1 is live-model and runs on a schedule; tier 0 evidence recorded. |
| Live evidence for the support claims | `tests/native/*`, `docs/evidence`, `docs/validation` | G0: evidence describes older code; five matrix cells never exercised (concurrent children, SW2 coalesced warning wake, Codex TUI children write-absence, ht-910 daemon restart with agent in pane, P40 crash fix3 real-host acceptance). Owned by ht-p03.20 (native matrix rerun) and ht-p03.37 (closure ledger). Not closed here. |
| F6 restored-pane creation vs repair reservation, BOTH-harness gate ht-910 | design preserved as unresolved dissent | Still unresolved by design; owner decision at finish. |

## 7. Residual items and closures

- Two remainder-capped roast candidates (`2026-10-01-...-roast-pr-1.md:6` "remainder-capped: 2", `:143` "2 candidates dropped by the remainder cap"): `git grep -n -i remainder archive/herdr-threads-run-2026-09-26` finds only the count; the candidates themselves were never written to any file in the tag. Closed as unrecoverable. The round-2 roast reports `remainder-capped` as none.
- Not read line by line: `src/protocol/pagination.rs`, `src/store/service_controls.rs`, `src/protocol/service.rs`, `src/store/invitation_due.rs`, `scripts/demo-tea-party.sh`, the tail of `scripts/validate-package.sh`. No bead, parked item or concern names them.
- Filed issues: ht-p03.121 (RD-1, Minor), ht-p03.122 (RD-2, Nit), ht-p03.123 (RD-3, Nit). Blocker issues filed: none.
