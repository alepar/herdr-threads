status: completed with 0 unresolved Blocking, 1 escalations [degraded: low coverage, coverage review advisory (method substituted), coverage review 2/3 seats for attribution+store and human goal-vs-tree read-through not done, code findings parked, final review: Should-fix (23 confirmed; converged)]
metrics: docs/superpowers/runs/2026-09-26-herdr-native-mailbox-thread-plugin/upstream-feedback-draft.md (draft not filed: addressed upstream in v6.4.2-alepar4.3, 5c92bce)

## Implemented

*Sourced from:* the beads tracker (`bd list --all --json --limit 0`, `bd show`), `run.md`'s phase-6 `codeBuckets` refresh, `fixBeads-round-1` and `sweep:` lines, `git log main..HEAD`, and `docs/validation/integration-sweep.md` (refreshed at `06773f5a`). Run-relative paths below are under `docs/superpowers/runs/2026-09-26-herdr-native-mailbox-thread-plugin/`.

**Bottom line for the merge decision.** All the planned work landed. The tracker has 128 `ht-4is*` beads (root plus 127 descendants: 81 task, 21 bug, 13 feature, 13 epic), and every one is closed. The four blocker beads outside the tree (`ht-fy0`, `ht-3xy`, `ht-z7j`, `ht-910`) are closed too. `codeBuckets.escalated` and `codeBuckets.pendingRetry` are both empty. The full serial suite passes at the code tip `ad905554` (after the regression-only pass); later commits touch only docs, so the stamp holds for this tip.

The branch is still not "clean":
- the code roast ended at Should-fix with 23 confirmed findings; the two that were regressions introduced by round-1 fixes were then fixed in one regression-only pass (no re-roast, per super-auto 6.4.2-alepar4.2), and the other 21 were parked;
- one design-roast escalation (F6) never got a verdict;
- every live-model native PASS was stamped on an older SHA (G0).

**Range.** `main@7c835439..ad905554` (code tip) plus docs-only record commits — 1073 commits in total — from 2026-09-26 to 2026-10-01. Design and planning come first; implementation, validation and UX run up to `384a663c`, where the pre-roast sweep passed. The round-1 roast fix loop is `44bda976..06773f5a`; the round-2 regression-only pass is `913228b8..ad905554`. After that come records and docs only.

**What landed, by epic.** All beads are closed. The merge SHAs come from `run.md`'s landing entries.

| Area (epic) | Children | What it delivers | Key landings (from run.md) |
|---|---|---|---|
| Contracts `ht-4is.1` | leaf | compiled component and API contracts (`src/ports.rs`, `src/protocol/`) | early wave; shared amendment rev 4 (`./shared-contract-amendment-adopted.md`) |
| Durable store `ht-4is.3` | 3.1–3.12 | SQLite schema (user_version 9), threads, membership, message snapshots, atomic receipt settlement, bounded history, deadline scans, warning/receipt indexes, retirement `last_error`, plus the already-joined invite bug (3.12) | C3 fan-in `98dc185`; `last_error` fix2 `77980f4`; Task10 accepted at `a0ca1100` |
| Seats and reconciliation `ht-4is.4` | 4.1–4.7 | Herdr host adapter, seat reconciliation, cooperative top-level contexts, check-in and operator repair, request budgets, observed-move fix (4.7) | host-seats `0419513`; recovery fixes, then host suite 12/12 at `fcb37c8` |
| Scheduling and wake `ht-4is.5` | 5.1–5.6 | bounded deadline scans, attention selection and retry, reservations, occupant-scoped warnings, WorkerStatus redaction, **production safe idle wake (5.6, P0)** | redaction `63a2581`; safe wake `979d25b` |
| Daemon `ht-4is.6` | 6.1–6.7 | runtime paths and exclusive ownership, bounded versioned IPC, ensure/run/stop, health, detached diagnostics, request-latency fix under load (6.7) | `17ecf62` (diagnostics) |
| CLI `ht-4is.7` | 7.1–7.4 | typed command dispatch, bounded progressive output and operator view, durable mutation intent journal, output encoder | CLI integration `84c9563`; lifecycle UX `008ac2d` |
| Harness adapters and UX `ht-4is.8` | 8.1–8.20 | Codex and Claude hook adapters, owned setup and unsetup, managed launch, plus the user-trial features: user-level setup with instance auto-detect, operator mode, pane names, short IDs, `skill`, `read --follow`, token diet, readable agent names, cooperative-mode health, Codex sandbox writable roots (8.20, P0) | recipes `d823d31`; Codex fingerprint `cb348bd`; Claude 2.1.286 `20d6288`; managed Codex `6111024`; burst accept `f24cac9` |
| Production wiring `ht-4is.9` | leaf | composition across component boundaries | `84c9563`, `ed6d242`, `88ca726`, `f4d1577` |
| Packaging and docs `ht-4is.10` | 10.1–10.9 | manifest, docs and CI, package lifecycle gate, support claims, clippy `-D warnings`, human output, release workflow and `curl \| bash` installer, README with screencast | clippy `267b300`; claims `f77750e`; package gate PASS at `d635aca` |
| Validation `ht-4is.11` | 11.1–11.12 | crash/concurrency matrix, native validation driver, Codex and Claude delivery matrices, host recovery suite, evidence report (`docs/validation/report.md`, PASS_WITH_GAPS), flake fixes | crash matrix `8c4206c`; driver `0b6d994` |
| Programmatic flows `ht-4is.30`/`.31`/`.32` (+ `.27`–`.29`) | 30.1–2, 31.1–3, 32.1–3 | persistent system registration, service-managed threads and required invitations, programmatic client, D2 system operations end to end | D2 `f4d1577`; required membership PASS both harnesses (32.3) |
| Native receipt workflow `ht-4is.2` + `ht-910` | 2.1–2.3 | cooperative top-level accept/ACK demonstrated in both harnesses (initial, restart, resume, Claude `/clear`, Codex `/new`, child delegation, idle wake, lost prompt) | Claude demo 4 PASS 31/31; Codex demo 3 PASS 26/26; matrices w11/w12/w23 |
| Design-roast spec fixes | `ht-4is.13`–`.25` (round 1), `.26` (round 2) | 14 specification-only beads amending the design docs | `./roast-design-1-applied-disposition.md` |
| Integration sweep `ht-4is.12` (+ 12.1) | 12.1 | R1–R25 traceability (MET 14 / MET-WITH-GAP 11 / NOT-MET 0 at `ad905554`) and the parked-findings disposition | sweep `54bd8040` |
| **Code-roast fix loop, round 1** | `ht-4is.33`–`.36` | seat lookup by pane uses a target-filtered query (`eed544e7`); body paging returns the complete remainder (`44bda976`); CLI and store share the 64 KiB body limit, and typed rejections are discarded (`020055fa`); a dead worker lane degrades Health (`1ffc113a`) | merged `06773f5a`, `a623ae86`, `f813875f`, `8030b5e2`, each with check and clippy passing |
| **Regression-only pass after round 2** (`run.md` `regressionPass-round-2`) | `ht-4is.37`, `ht-4is.38` | durable intents are discarded only on an allow-list of deterministic rejections (InvalidRequest, Unauthorized, Archived, Conflict, OperationPayloadMismatch, MembershipRequired); transient codes and NotFound keep the intent (`913228b8`, `0a2ecbfa`); the hook and CLI share one resolved-first pane seat collector that pages past unresolved seats (`6def6571`) | merged `de0c4e0e`, `2be3b98c`, each with check and clippy passing; not re-roasted |

**Verification of record** (from the latest `sweep:` line in `run.md`): `cargo test --locked --all-features -- --test-threads=1` at `ad905554` gave a PASS: 14 result lines, lib 1217 passed / 16 ignored, 0 failed. The first sweep after the regression pass (at `2be3b98c`) failed one existing test, because the ht-4is.37 allow-list also changed seat resolution; `ad905554` scoped it to cooperative intents. That sweep is the only check the regression pass got beyond focused tests. The suite is only known to pass serially (see Smells). Native evidence is in `docs/validation/report.md` (verdict **PASS_WITH_GAPS**, report base `99ea74ca`). The four 2026-09-30 revalidation runs at `e6ac1985` passed on both harnesses (`./native-matrix-w23/`).

## Remaining

*Sourced from:* `run.md`'s `parked:` block, `scopeFilter-round-1` block, `fixLoopExit` and `integrationSweep` lines, and its `codeBuckets` refresh; the roast reports `./2026-10-01-herdr-native-mailbox-thread-plugin-roast-pr-1.md` and `-roast-pr-2.md`; `docs/validation/integration-sweep.md` (gaps B1–B4, G0); `docs/validation/report.md`.

**Why the status line reads this way.**
- *0 unresolved Blocking.* Round 2 has 0 Blocking, and no `roastDesignCapped` or `roastCodeCapped` record exists.
- *1 escalation.* `codeBuckets.escalated` is empty, but the design-roast escalation F6 is still parked in `run.md`.
- *Degraded qualifiers*, one per source:
  - `low coverage`: design roast 3's parked degraded-verdict.
  - the two coverage-round-1 qualifiers: two parked degraded-verdict records. The second also records that the mandatory human goal-vs-full-tree read-through was never done.
  - `code findings parked`: `codeBuckets.parked` is literally `[]`, but the same line points to `./parked-findings.md`, where review findings that cleared the per-task gate were merged as parked.
  - `final review`: `codeBuckets.review` is the round-2 PR roast verdict, not CLEAN.
- *No sweep qualifier.* The sweep passed on the post-regression-pass tip (see `run.md`'s latest `sweep:` line).
- `pendingRetry` is empty, `stalled: false`, no graph-change records exist, and the round-1 step-back was `patch`, so no redesign was proposed and left unapplied.

### Fixed after convergence, not re-reviewed: two regressions the fix loop introduced

`run.md`'s `fixLoopExit` line flags these, and roast round 2 confirms both at Should-fix 3/3. Each round-1 fix fixed its original finding and opened a new defect in the code it replaced. After the run switched to skill source `4cd542b` (`run.md` `skillSource:`), both were fixed in the one regression-only pass that version prescribes (`regressionPass-round-2`, beads `ht-4is.37`/`.38`, merges `de0c4e0e`/`2be3b98c`). By design nothing re-roasts that pass; the fixes carry focused tests only. Original findings, for the record:

1. **[Should-fix] `src/cli/retry.rs:119` (introduced by `020055fa`, the body-limit fix).** Every cooperative mutation now deletes its durable journal intent on *any* correlated daemon error except `UnknownOutcome`. That includes transient, retryable codes (`StoreBusy`, `DeadlineExceeded`, `Cancelled`) and rejections that arrive after send-preparation rows have already committed. This breaks the CLI design's no-discard durable-intent rule (cli-design.md:85, :87). It had downgraded R16 to MET-WITH-GAP; R16 is MET again at `ad905554`. *Fix shape:* discard only an allow-list of deterministic codes and add a correlated `StoreBusy` test.
2. **[Should-fix] `src/cli/hook.rs:734` (introduced by `eed544e7`, the seat-lookup fix).** The hook and the CLI pick a pane's seat by different rules. After an operator repairs a pane with `seat resolve --new-seat --operator` (or rebinds onto it) while an old unresolved seat still points there, the hook refuses check-in for the new resolved seat on every call. This breaks the operator repair path the design provides (R3). *Fix shape:* filter `state='resolved'` server-side and use one shared lookup.

### Parked escalation (no verdict)

- **F6** (`./2026-09-27-herdr-threads-roast-design-1.md`, escalation): restored-pane creation versus repair reservation. The panel's dissent on restore-hold allocation was never re-reviewed. The root design (line 36) and `integration-sweep.md` R3 both still carry it as open. A human should either accept the explicit restore-hold clarification or ask for the fresh review the escalation required.

### New in roast round 2, parked on convergence

- **[Should-fix] `src/store/seats.rs:1217`:** the observation worker walks every seat ever created, retired ones included, 16 per ~100 ms, after each snapshot. Reconciliation latency therefore grows linearly with lifetime seat count (about 5 s at 800 seats). *Fix shape:* one SQL filter, `state != 'retired'`.

### Punch list from scope filter round 1 (20 findings, all still open in round 2)

Each finding is confirmed in round 1, carried unchanged into round 2, and listed here with the reason recorded in `run.md`'s `scopeFilter-round-1` block.

1. [Should-fix] `src/store/mod.rs:838`: wake worker rebuilds each seat's attention from full retained history every cycle. **out of scope (filtered)**: Wake-attention rebuild cost growing with history is a performance/scaling quality issue; results stay correct.
2. [Should-fix] `src/store/seats.rs:702`: a snapshot generation plus per-pane rows is written every ≥5 s and never pruned. **out of scope (filtered)**: Unbounded snapshot retention growth is a resource/retention hardening concern the goal does not name.
3. [Should-fix] `src/identity/reconcile.rs:885`: failed host snapshots retry every ~100 ms with no backoff. **out of scope (filtered)**: Missing backoff on observation retries is efficiency hardening, not incorrect goal-named behavior.
4. [Should-fix] `src/store/mod.rs:689`: work-job discovery scans completed rows, and completed jobs are never deleted. **out of scope (filtered)**: Work-job discovery scanning completed rows is a scaling inefficiency; discovery remains correct.
5. [Should-fix] `src/main.rs:52`: daemon startup failures before or after the log sink go to /dev/null; `ensure` sees only a timeout. **out of scope (filtered)**: Startup stderr diagnostics are an operability improvement outside the goal-named behaviors.
6. [Should-fix] `src/ports.rs:1292`: the allocation, revocation and `CallerVerifier` ports have no production callers (`allow(dead_code)`). This is sweep blocker **B1**, and it leaves `ht-4is.12`'s "no inert production port" acceptance unmet. **out of scope (filtered)**: Unused port code and dead-code allowances are a code hygiene issue, not a correctness defect.
7. [Nit] `src/service/workers.rs:857`: materialization is capped at 1 job/s, and the "tick after committed change" is not wired. **out of scope (filtered)**: Materialization throughput and tick wiring is a performance improvement, not a goal-named correctness defect.
8. [Nit] `src/store/queries.rs:990`: page sizing re-encodes the whole page per candidate (quadratic). **out of scope (filtered)**: Quadratic page sizing is a performance quality issue with correct output.
9. [Nit] `src/daemon/transport.rs:67; :68`: cancellation is a 10 ms sleep-poll, so the idle daemon never quiesces. **out of scope (filtered)**: Idle polling efficiency is not a goal-named behavior.
10. [Nit] `src/store/connection.rs:550`: unlisted SQLite errors map to `StoreCorrupt`. Round 2 notes this now also discards durable intents via item 1 above. **out of scope (filtered)**: Error-code mapping precision is diagnostics polish outside the goal.
11. [Nit] `src/service/workers.rs:625`: worker failure detail lives only in an unread 8-entry in-memory ring. **out of scope (filtered)**: Diagnostic logging of worker failures is observability improvement not named by the goal.
12. [Nit] `src/service/workers.rs:599`: a poisoned WorkerStatus mutex reads as Ready. **out of scope (filtered)**: Poisoned-mutex health edge case is observability hardening, unrelated to a goal-named behavior.
13. [Nit] `src/cli/journal.rs:467`: `allocator.lock` is opened without `O_NOFOLLOW` in a sandbox-writable directory. **out of scope (filtered)**: O_NOFOLLOW hardening for a sandbox symlink case is security hardening outside the goal and the stated cooperative-identity contract.
14. [Nit] `src/daemon/lifecycle.rs:181`: version skew in the `ensure` handshake surfaces as a timeout, not `daemon_version_mismatch`. **out of scope (filtered)**: Version-skew error mapping in the ensure handshake is not goal-named behavior.
15. [Nit] `.github/workflows/ci.yml:101`: tests that fail under the parallel harness are waived, and CI forces `--test-threads=1`. **out of scope (filtered)**: CI test-harness hygiene, not a goal-named behavior.
16. [Nit] `tests/store/attention.rs:277`: a process-global VM-instruction counter corrupts cost-flatness tests under parallel runs. **out of scope (filtered)**: Test isolation of cost-flatness tests is test hygiene, not a failing test for a goal-named behavior.
17. [Nit] `Cargo.toml:1`: the project declares no license (no LICENSE file, no `license` field). **out of scope (filtered)**: Missing license is repository/release hygiene outside the goal's behaviors.
18. [Nit] `scripts/package-release.sh:46`: release archives carry no third-party license notices. **out of scope (filtered)**: Third-party notices in release archives are packaging compliance, not a goal-named behavior.
19. [Nit] `docs/install.md:67`: about 74 MB / 5,194 files of run evidence plus 11.7 MB of media are committed without LFS. **out of scope (filtered)**: Committed run evidence bloat is repo hygiene.
20. [Nit] `docs/superpowers/runs/2026-09-26-herdr-native-mailbox-thread-plugin/task-tree-roast-design-1.json:10`: committed run evidence carries the owner's email (13 files), absolute `~` paths (~750 files) and PATH values. **out of scope (filtered)**: Personal data in committed run evidence is repo hygiene unrelated to goal-named behavior.

Items 17–20 were fixed before the merge, at the human's choice (`run.md` `finish:`): dual MIT/Apache-2.0 license plus generated third-party notices in every release archive; run evidence trimmed from 74 MB / 5,199 files to a curated, scrubbed 1.4 MB set (`docs/design/`, `docs/history/`, `docs/evidence/`), with the full original in local tag `archive/herdr-threads-run-2026-09-26`; main received a squash merge so its history carries none of the removed data. Sweep after hygiene: PASS at `28992889`.

### Seen in round 1 but never filed or judged

From `./2026-10-01-herdr-native-mailbox-thread-plugin-roast-pr-1.md`:
- 19 spot-check-confirmed Nits under "Unverified nits (spot-checked)", for example release actions pinned to mutable tags (`release.yml:128`) and the installer's upgrade-time stop skipped when the Herdr server is down (`install.sh:377`). The scope filter never received them, so they are neither fixed nor punch-listed.
- 2 candidates dropped by the remainder cap ("Beyond remainder cap"). Nobody saw them.

### Validation gaps (`docs/validation/integration-sweep.md`, `docs/validation/report.md`)

- **G0 / B2, native evidence is stale.** Every live-model and real-host PASS describes the code at its own SHA. `06773f5a` differs from the report base `99ea74ca` in 60 more source files; the package gate (`d635aca7`) predates 65 changed source files, and the host suite predates 22. Before any native row becomes a release claim, it has to be rerun on the release SHA.
- **B3, owed native cells** (report verdict PASS_WITH_GAPS):
  - concurrent children: NOT_EXERCISED on both harnesses;
  - coalesced warning-only wake (SW2): NOT_EXERCISED live on both harnesses, covered only by the stand-in host R13;
  - Codex TUI children write-absence: NOT_EXERCISED.
- **B4.** The open round-2 Should-fix items map to R17, R18 and R20 (R3 and R16's items were fixed in the regression pass).
- **Versions without evidence.** No receipt runs exist for recipe versions Claude 2.1.283/2.1.284 or Codex 0.157.1/0.158.0. Codex 0.159.2 is admitted only as "schema-matched, live-unverified". Claude managed launch was last run on 2.1.285.
- **Release readiness** (`./parked-findings.md` wave 21):
  - the Linux musl and Intel macOS builds have never been compiled;
  - the manifest still declares `platforms = ["macos"]`;
  - `release.yml` creates a GitHub release on a manual dispatch from a tag ref, which contradicts `docs/release.md`.
- **Not measured:** token overhead has no matched benchmark.

## Gotchas & surprises

*Sourced from:* `run.md` (design-roast dispositions, checkpoints, user decisions, `stepBack-round-1`), `./2026-10-01-herdr-native-mailbox-thread-plugin-roast-pr-1-step-back.md`, `./roast-design-1-applied-disposition.md`, `./cooperative-receipt-user-direction.md`, `./closeout-audit-2.md`, `./friction.md`, `./task10-integration-quarantine.md`, and the close reasons of `ht-fy0`, `ht-3xy`, `ht-z7j`, `ht-910`. `codeBuckets.slowness` was never recorded, and the ledger (`.superpowers/sdd/ht-4is-plan/progress.md`, last written 2026-09-28) has no `Slowness:` or `Edge cut:` lines. The slowdowns below come from `run.md`'s narrative entries.

**The design changed in four places.**
- **Design roast 1 was Blocking (13 confirmed: 7 Blocking, 6 Should-fix).** It forced a coherent architecture-contract redesign: pagination and continuation for every collection, whole-call budgets and bounded lanes, fresh execution evidence per accountable mutation, ordering fences between host observation and identity transitions, deciding-transaction deadline time, and recording warnings before retirement. These became 13 spec-fix beads (`ht-4is.13`–`.25`). Round 2 converged with 1 Should-fix (bounded retirement, `ht-4is.26`). Round 3 was clean but `[low coverage]`.
- **Caller attribution went from adversarial to cooperative** (user direction, 2026-09-28, `./cooperative-receipt-user-direction.md`). Service-side proof that a caller is the top-level agent stopped being a prerequisite. Receipts carry `cooperative_top_level` provenance: a prompted claim joined to a transcript root call. This is what let `ht-910` and `ht-4is.2.3` close. It also means DB-level child separation is an accepted limit (B1 in the validation report): a child that ignores instructions can still ACK.
- **The hook check-in write model was redesigned twice.** First, tool-boundary hooks became non-durable and coalesced. Then, after the hook fix failed completeness twice, attention moved to a server-side digest. That lane ran past its 5-round breaker and merged at fix6.
- **Setup went user-level only** (user decision, 2026-09-30, `ht-4is.8.8`): `~/.claude/settings.json` and `~/.codex/{hooks.json,config.toml}`, with project mode removed.

**Reality diverged from the design.**
- A composition probe at `7c47f88` (`./composition-probe-1/`) showed that, on a real Herdr host, nothing worked end to end even though component tests were green:
  - native incarnation was always unknown;
  - the seat-resolve epoch disagreed between store and adapter;
  - there was no hook stdin entrypoint, and the installed hook exited 2;
  - operator mutations were Unsupported;
  - lifecycle UX was broken.

  Composition wave 1 was needed to fix this.
- In production, idle wake did not exist: `safe_wake_target` was hard-wired to None. Herdr reports a finished turn as `done`, not `idle`. This was found only by the live Claude TUI demo and fixed as P0 `ht-4is.5.6`.
- The Codex workspace-write sandbox blocks the daemon socket (EPERM). `setup codex` now emits a narrow socket allowance. Later, every native Codex run had used a `/private/tmp` state directory, so all of them missed that real installs cannot write `~/.local/state` from the sandbox. This was P0 `ht-4is.8.20`, found in the user's live trial.
- Cooperative bindings were stored without terminal_id/incarnation, so one host socket denial left every cooperative seat permanently unresolved. The crash matrix found this (`src/store/seats.rs:3475` at the time).
- The installed harnesses kept moving mid-run: Claude auto-updated 2.1.284 → 2.1.285 → 2.1.286, and Codex 0.159.2 → 0.159.3. This forced a recipe registry with version intervals and schema-fingerprint admission of unlisted Codex versions.

**Blocker beads, all triaged and closed.**
- `ht-3xy` / `ht-4is.3.5`: Task10 merge quarantine after repeated fixture conflicts. Released only by a user-approved exception to the conflict-resolution cap (`./cooperative-receipt-user-direction.md`).
- `ht-z7j`: a Task6 cursor-fixture conflict.
- `ht-fy0`: the cooperative core merge gate.
- `ht-910`: native caller attribution, closed under the cooperative contract. `./closeout-audit-2.md` had listed a live-model "daemon restarts while the agent stays in its pane" run as missing for `ht-910`. The final close reason does not mention it, and the sweep's daemon-restart coverage uses stand-in seats with no model.

**Code-roast step-back versus scope filter.** `stepBack-round-1` decided `patch`. It found three clusters (discovery not using pending/indexed projections, silent lane health, fixed-interval polling) and asked for each cluster to be swept consistently. The scope filter then punch-listed most of those same cluster members as performance or observability hardening. So the step-back's "apply the rule to every list/discovery query in one pass" was not carried out. Only `hook.rs:716` (cluster a) and `app.rs` (cluster b) were fixed, and the `hook.rs` fix regressed.

**Process and slowness** (from `run.md` and `./friction.md`).
- Under the original rules, every NONZERO review triggered a full cumulative audit of all prior reviews. These audits grew past 270 reports, about 1 MB each time, and dominated phase 3 until the user relaxed the policy on 2026-09-30.
- The run switched skill definitions mid-run (`skillSource` → `e8244b0`). The `migrated:` lines record which counters became history.
- A per-merge build check caught 3 real cross-branch compile seams (`b0d24c5`, `80f90cc`, `bf00d19`) that lane tests could not see.
- One merge-back reported "check pass" on a no-op merge because an untracked evidence copy blocked it. It was recovered, and the merge guard was tightened upstream.
- The run moved from Codex to Claude Code on 2026-09-29.
- The Codex `default` profile hit its usage limit, so the Codex demos switched profiles.
- **Environment side effect:** each Codex run wrote trust entries into the user's `codex-1` `config.toml` (2574 → 2993 B, approved for scratch projects only).

## Entrypoints

*Sourced from:* the task tree's dependency order (beads `ht-4is.1` → `.3` → `.4` → `.5`/`.6` → `.7`/`.8` → `.9` → `.10`/`.11`), the design docs (`./2026-09-27-herdr-threads-design.md` and its eight `--*-design.md` component specs), `README.md`, and `docs/validation/integration-sweep.md`'s implementation column.

Read in this order:

1. **The goal and contract.** Start with `./2026-09-27-herdr-threads-design.md` and `./shared-contract-amendment-adopted.md` (revision 4, normative for types, schema and ownership). Then read `README.md` §"What an ACK means" and §"Trust model: cooperative, not enforced". Those two sections explain why receipts are claims, not proofs.
2. **The root module: contracts.** `src/ports.rs` defines the component ports (`StorePort`, `HostPort` and the others), including the four B1 constructors that production never reaches. `src/protocol/` holds the wire types, authority/permits, pagination and IDs.
3. **The durable core: store.** `src/store/mod.rs`, `src/store/seats.rs`, `src/store/messages.rs`, `src/store/queries.rs`, `src/store/effective.rs`; migrations are under `migrations/`. Most of the open Should-fix findings live here (`mod.rs:689`, `mod.rs:838`, `seats.rs:702`, `seats.rs:1217`, `queries.rs:990`).
4. **Identity and host.** `src/identity/reconcile.rs` and `repair.rs` (seat reconciliation and the observation lane), and `src/host/native.rs` (the Herdr adapter and `safe_wake_target`).
5. **Scheduler and service.** `src/scheduler/deadlines.rs`, `src/service/workers.rs` (the background lanes and `lane_guard`), and `src/service/dispatch.rs` (how a request reaches the store).
6. **The daemon.** `src/daemon/lifecycle.rs` (ensure/stop), `transport.rs`, `health.rs`, and `ownership.rs`.
7. **The primary callers: CLI and hooks.** `src/cli/mod.rs` (command routing, `pane_seat`), `src/cli/hook.rs` (hook entrypoint, `find_seat`; open finding at :734), `src/cli/retry.rs` and `src/cli/journal.rs` (the durable intent path; open finding at retry.rs:119), and `src/cli/setup.rs` and `src/cli/launch.rs`. Harness recipes and adapters are in `src/harness/` (`recipe.rs`, `claude.rs`, `codex.rs`, `codex_schema.rs`).
8. **Wiring.** `src/app.rs` (production composition and health) and `src/main.rs`.
9. **Evidence.**
   - `docs/validation/integration-sweep.md` maps R1–R25 to code and tests.
   - `docs/validation/report.md` holds the native matrix.
   - `tests/integration/sweep.rs` drives the installed binary end to end.
   - `scripts/validate-native-demo.py` is the live-model driver.

## Smells

*Sourced from:* `./parked-findings.md` and `./parked-disposition.md` (code-review minors parked during super-code), `run.md`'s `parked:` degraded-verdict records and process-policy entries, the `DONE_WITH_CONCERNS` implementer reports under `.superpowers/sdd/ht-4is-plan/` (git-ignored, in this worktree), and the round-2 roast report.

- **The fix loop regressed, and the fixes to the fixes were never re-reviewed.** Two of the four round-1 fixes introduced new Should-fix defects (`retry.rs:119`, `hook.rs:734`; see Remaining). The regression-only pass fixed both, with focused tests, and by design no roast re-checked them. *The smell:* the deterministic-code allow-list in `src/cli/retry.rs` (`is_deterministic_rejection`) and the shared seat collector in `src/cli/mod.rs` (`collect_pane_seats`) are reviewed only by their own tests.
- **Merged with fix passes that were never re-reviewed.** From the user's 2026-09-30 light-review policy onward (`run.md` "User process policy update"; "Wave 6 landed … one review and one fix pass for its Important findings"), each lane got one relaxed review and at most one fix pass, and then merged without re-review. Before that, lanes merged only after a CLEAN re-review. *The smell:* roughly the last third of the feature work, including user-level setup, short IDs, token diet, human output, release/installer and `read --follow`, was checked by one reviewer plus the final roast.
- **About 110 parked minors were never dispositioned.** `./parked-disposition.md` dispositions the first 100 entries: 10 FIXED, 9 FIX-NOW (since fixed), 40 ROAST, 41 WAIVE. Waves 15–31 were parked afterwards (`git log` on `./parked-findings.md`: 2026-09-30 21:14 → 2026-10-01 02:53), and nobody dispositioned them. They include:
  - the release-gate and installer issues;
  - `features.network_proxy` keys widening the Codex allowance;
  - wrong state-dir auto-detection with no error;
  - `me init` recording agent actions as `operator_human`;
  - `read --follow` retrying forever on persistent errors;
  - a 48-bit pagination tag replacing exact equality;
  - shared sandbox-writable `intents/` and `contexts/` across seats.

  *The smell:* a judge argued against each of these, and the code merged anyway, with no recorded verdict.
- **40 ROAST-dispositioned parked items** (for example P10 "adapter never reports `EmptyShell`", W6-C3 "top-level `resume` accepted without live evidence", W9-1 "retry ladder can delay a wake 5 min", and the validator false-PASS paths P15, W10-E1 and W10-E7) were sent to the code roast as context. The roast did not re-surface them as confirmed findings. *The smell:* "carried to the roast" became "not mentioned by the roast", which is not the same as cleared.
- **The test suite is only green serially.** `hook_entrypoint` exits 101 under the default parallel harness. `ht-4is.11.8` was *waived*, not fixed. Several store, journal and service tests flake under load (P12, P13, wave 27–30 notes), and CI pins `--test-threads=1` (punch-list item 15). *The smell:* a plain `cargo test` is not a trustworthy signal, and nobody has root-caused the crash.
- **Production ships an unreachable verification layer** (B1, `src/ports.rs:1292`). The allocation, revocation and caller-verifier paths are tested but have no production caller. *The smell:* the tests exercise a design that production does not run.
- **Degraded-verdict records** (`run.md` `parked:`):
  - design roast 3 returned zero candidates and was labelled `[low coverage]`;
  - coverage round 1 used a substituted bounded-chunk reviewer method, and two scopes got only 2 of 3 reviews;
  - the mandatory human goal-versus-full-tree read-through was parked and never done.

  *The smell:* autonomous mode answered these gates itself, so a human never checked that the 126-bead tree covers the goal.
- **17 implementer reports ended `DONE_WITH_CONCERNS`** (for example task-3, -7, -15, -19, -20, -23, -25, -26, -32, -44, -45, comp-host-seats, claude-sessionstart-setup, host-recovery-validation, and the three cooperative-harness reports). Most of the concerns were provisional bases or open native gates that later closed. *The smell:* nobody re-checked them one by one at close-out.
- **UX items left uninvestigated** (`./parked-findings.md` waves 25 and 31):
  - Herdr's agent list shows the name `None` for some launched agents once they are working, even though launch records the name. Nobody has checked whether Herdr clears it.
  - The daemon keeps reporting the Codex version it saw at start after the binary updates.
  - A wake prompt was once left typed but unsent in a Claude guest's composer (demo take 6).
- **Repository hygiene.** Run evidence makes up about 74 MB of the repository, includes personal data, and is committed (punch-list items 19–20). This report and its run directory add to that.
