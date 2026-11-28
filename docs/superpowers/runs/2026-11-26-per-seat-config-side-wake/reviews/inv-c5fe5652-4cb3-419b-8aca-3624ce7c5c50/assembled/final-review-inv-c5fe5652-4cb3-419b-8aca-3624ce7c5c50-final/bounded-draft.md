# Final review (bounded): ht-z41e, per-seat config and Codex /side queue wake routing

Captured source: base 236c77a660c3bb19fb31472007d8b0c670f3b907, head d323e79c17380623ea10ec398f23ecd96529bf74
(both verified with `git rev-parse --verify <sha>^{commit}`). Branch super-auto/per-seat-config-side-wake, read from
captured Git only (`git diff`, `git show <head>:<path>`); no worktree was created, nothing was built or run.

## Route and scope

- Bounded route: `review-inputs.json` reads status unavailable, sourceAvailable true, wholeRangeReady false, 764
  changed, 0 omitted, 2 gaps (run.md:128 inv-f5f76bdd: incomplete claimed collection; incomplete split bundle). The
  failed assembly and sidecar were left as found.
- 764 changed paths, about 505k inserted lines. About 557 paths and 492k lines are run evidence and review packages
  under `docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/` (about 103 MB in the head tree); the rest is
  Rust source, tests, scripts and docs.
- Inspected: every production Rust hunk of the seat-config path (registry, CLI parse and dispatch, service routing,
  store, migrations, threshold column write, retirement cleanup), queue selection and arm, the queue runtime, the
  dispatcher, the scheduler's receipt check, delivery-pull contacts, the native host's queue admission, the Codex
  adapter (root marker, composer), the bridge claim plumbing, capability gating, settings, doctor and the ports; also
  TRUST-POLICY.md, AGENTS.md, SKILL.md, agent-usage.md, CHANGELOG.md, the design doc and qualification.md. The
  coverage JSON lists the exact hunks.
- Not inspected: test files (except a7d1d92e0's changes to tests/store/wake.rs), scripts (wake-e2e.py, selftests,
  release-qualify), fixtures, and the evidence trees.

## Requirement baseline trace (design §Requirement baseline)

- B1 (replace node setting with seat setting): delivered. `InstanceSettings.programmatic_wake` is `Option<bool>` and
  has no effect (`src/daemon/settings.rs`). `ServiceConfig` and `QueueWakeRuntime` lost the flag (`src/app.rs`,
  `src/service/config.rs`, `src/notification/queue.rs`). `queue_wake_selection` and `arm` read
  `seat_config::programmatic_wake_on` in their transactions and fail closed. `doctor` prints the obsolete line and
  lists the seats that have the key on.
- B2 (generic key=value capability, closed registry per S1): delivered. `src/protocol/seat_config.rs` `REGISTRY` with
  typed values, tier and backing. Table rows plus the column-backed `nudge_threshold`. Exact validation, refusals
  name the valid set.
- B3 (CLI that reads and writes): delivered. `ht config seat get|set|unset`, `ht human config seat set|unset`
  (`src/cli/commands.rs` `Top::Config`, `src/cli/config_seat.rs`), daemon commands `GetSeatConfig`,
  `SetSeatConfig`, `UnsetSeatConfig` behind `seat.config_v1`.
- B4 (CLI listing keys with descriptions and values): delivered. `config seat list` is static, no daemon, has a
  `--json` form and a trailer naming both write spellings.
- B5 (telling an agent in prose works): delivered. SKILL.md and agent-usage.md point at `config seat list`. The live
  config-prose row recorded `config seat list` followed by `human config seat set --seat <seat> programmatic_wake on`
  and store value `on`. That run used `--approve-for-me`, so the ask prompt was bypassed.
- B6 (human-only writes for some keys, every key readable, low-risk keys writable by agents): delivered. An
  agent-tier write of a human key is refused at parse time (`refuse_agent_config_write`) and again in the store
  (`agent_write`). `GetSeatConfig` is a read with no claim. The human tier is an operator decision
  (`OperatorActor::from_peer`). Ask rules are pinned for Claude and Codex. The accepted limit for other harnesses is
  recorded in TRUST-POLICY.
- B7 (nudge threshold settable by an agent): delivered. `nudge_threshold` is agent tier. `agent_write` goes through
  `seats::write_threshold`, with a read-only binding refused. The aliases share `write_column`.
- B8 (wake reaches the root session while /side is open): code path present (`process_root` with
  `HarnessAdapter::wake_process_root`; Codex root rule over `concurrent_session` and `auxiliary_session`; root-scoped
  receipt; `admit_queue_wake`). Live result at the final SHA: PASS on a private Herdr server, FAIL on the shared one
  (wake outcome `unsafe`, cause undiagnosed). Partly delivered; see Must fix.
- B9 (no native typing into the side chat; the wake waits until you leave /side): the deferral holds live
  (`refused_while_side`, `side_typed: false`). The live `side-native-defer/group` row shows no root wake after the side
  chat closes. The approved request says the wake "then waits until you leave /side", so the second half is not
  delivered; see Must fix.
- B10 (brainstorm decisions): delivered as decided. Node key removed with no migration. Own-seat agent writes; the
  human tier may name any local seat. Threshold is a key over the existing column. Spikes ran on the codex-2 profile.

## Cross-task integration observations (own view, before the ledger)

- Seat-config writes compose correctly across tiers. Both tiers share one apply body. Column writes upsert an audit
  row and re-judge the wake. Table writes re-judge too, so a `programmatic_wake` flip also lifts `unreceived`
  through the `AwaitingReceipt.programmatic_wake` comparison. Final retirement deletes rows in both retirement-job
  paths. An operator write to a finally retired seat is refused with `seat retired: <seat>`.
- Commit a7d1d92e0 (ht-z41e.43) makes a lazy frontier seat-level attention in `load_candidate` and `reserve` for
  every harness, not only queue-capable seats. A native wake for a lazy item now reserves every connected top-level
  binding. Native composer admission's all-bindings-idle rule therefore also covers bindings the item was not owed
  to. The same commit changed two existing fixtures in tests/store/wake.rs (`accept_invitation` and
  `an_item_resolved_before_the_prompt_is_not_typed`) from `state='accepted'` to `state='already_joined'` to keep them
  green. The design's §Unchanged behavior does not list this widening.
- ht-z41e.43 added a second auxiliary-session evidence source: on a fork SessionStart with a path, the hook reads
  the rollout's first line for `thread_source: tui_side_chat` (`src/harness/codex.rs` `auxiliary_session`,
  `src/harness/attribution.rs` `transcript_head_json`). It also added a split-pane composer read for Codex's
  side-by-side layout (`split_panes` in `src/harness/codex/composer.rs`). TRUST-POLICY.md, AGENTS.md and the design
  doc do not mention either. TRUST-POLICY still says a /side session emits a null `transcript_path` on every hook
  event, which the 0.172 live runs contradict.
- With a persisted side rollout, a side first seen by a non-SessionStart event carries a path and gets no auxiliary
  bit. The process then names no root and the wake stays native. This fails closed: it loses queue routing, never
  hits the wrong target.
- `DeliveryPullState.session_contacts` is never evicted (one entry per seat and native session for the daemon's
  lifetime). `QueueWakeRuntime.roots` clears the whole map at 256 entries. After a clear, a queued attempt gets no
  `AwaitingReceipt` at all, because transport is Queue and the root is None.
- Doc numbering drift: TRUST-POLICY says `seat_config` is "NodeDelta 20" and the auxiliary bit is "node migration
  22"; the schema registers versions 21 and 23. `src/store/seat_config.rs` says "NodeDelta 0020". The registry doc
  says descriptions stay "under 110 bytes", while the test allows under 130.
- About 103 MB of run evidence and review assemblies are committed into the head tree, including an earlier
  final-review package's diff slices (`reviews/inv-f5f76bdd-.../slices/*.diff`). No credential patterns turned up in
  a grep: the hits were a test fixture's FAKE_API_KEY and a reviewer's prose.

## Ledger triage (after the own view)

- Task 23 (ht-z41e.31) `[confirmed-live-fail]` is graded minor (deferred) but records known product failures at the
  final SHA. It is misgraded and is triaged as must fix below.
- The `missing-evidence` cluster (×8 across 8 tasks) is process-class: report evidence kept in scratch or in prose.
  It is not a product defect. Its practical cost is the untested-scope items below.
- ht-z41e.7 `[mis-enrolled-side]` is superseded: the lone-binding rule now reads both `concurrent_session` and
  `auxiliary_session`.
- ht-z41e.45 `[doc-drift]` (side draft) is resolved: TRUST-POLICY now states that typed text refuses a queue wake.
- ht-z41e.1 `[unexecuted-test]` (herdr_release_wake) is resolved: run in ht-z41e.31 and passed on Herdr 0.9.5.
- ht-z41e.31 `[evidence-bulk]` (87 MB of evidence) is triaged as must fix below.
- The ledger has no parked lines and no BLOCKED-AUTH lines. stop-run-processes `--check` failed repeatedly (exit 4,
  git worktree inventory), so leak checks across tasks were done by hand.

## Findings

- [important] Live FAIL at the final SHA: `side-native-defer/group` gets no root wake after the /side chat closes (qualification.md, "Side cases"). Under delivery-contract rule 2, an idle root with a p0 pending must wake, and B9's "waits until you leave /side" is not met.
- [important] Live FAIL at the final SHA: `side-queue/group` fails on the shared Herdr server (wake outcome `unsafe`, not delivered in the root) and passes only on a private server; the cause is undiagnosed ("pane geometry"). B8 is delivered only conditionally.
- [important] Live FAIL: every row of the existing opt-in gates `side-nudge/group` and `side-draft/group` (ht-327) fails with `side_setup_ok: false` and wake outcome `unsafe`. No base-SHA measurement exists, so a product regression is not ruled out.
- [important] Live FAIL: `resume-idle/group` (an existing opt-in case) did not deliver the p0, and `flag-flip/group` observed no stale-authority refusal in either mode. Both are unattributed.
- [important] TRUST-POLICY drift: ht-z41e.43 added the rollout-head `thread_source: tui_side_chat` marker and the split-pane composer read. TRUST-POLICY still describes the marker as a null `transcript_path` on every side event, and the AGENTS.md trust rules require the policy to change in the same commit.
- [important] About 103 MB of evidence and review assemblies are committed under docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/. Once merged to main this is permanent history.
- [concern] a7d1d92e0 widens lazy-wake reservation targets to every connected binding for every harness, and two pre-existing wake.rs fixtures were changed from `accepted` to `already_joined` to stay green. This is outside §Unchanged behavior, and the native all-bindings-idle admission now covers unowed bindings.
- [nit] `session_contacts` is unbounded and `roots` clears all entries at 256; migration-number and byte-budget wording drift in TRUST-POLICY, seat_config.rs and the registry doc.

## Verdict

not ready (live wake FAILs at the final SHA, misgraded minor: B9 root wake after side close and B8 on the shared Herdr server; TRUST-POLICY drift; about 103 MB of evidence; no full-suite run)

## Must fix before landing

- B9 (baseline row): `side-native-defer/group` shows no root wake after the /side chat closes at the final SHA (qualification.md "Side cases"; ledger Task 23 `[confirmed-live-fail]`, misgraded minor). Diagnose and fix, or get a human scope decision.
- B8 (baseline row): `side-queue/group` fails on the shared Herdr server (`unsafe`, item not delivered in the root) and passes only on a private server. Diagnose the geometry-dependent refusal in the dispatcher or admission path and rerun on the shared server.
- `side-nudge/group` and `side-draft/group` (pre-existing ht-327 opt-in gates) fail every row with `side_setup_ok: false`. Measure them on base 236c77a6 to tell a driver defect from a product regression, then fix whichever it is.
- `resume-idle/group` (p0 not delivered) and `flag-flip/group` (no stale-authority refusal) fail live with no attribution. Measure them on the base and fix, or record an explicit user acceptance per AGENTS.md's wake e2e FAIL rule.
- Update TRUST-POLICY.md (the "Queue wake target with /side sessions present" section) and AGENTS.md to describe the rollout-head `thread_source: tui_side_chat` marker and the Codex split-pane composer read added by ht-z41e.43, and correct the "null transcript_path on every side event" claim.
- Trim the roughly 103 MB of committed run evidence and review-package slices (results.json, commands.jsonl, reviews/inv-*/assembled/slices) to the reports and manifests before landing on main, or get explicit human acceptance of the history cost.

## Untested scope

- No full-suite measurement of this branch exists; the caller runs `cargo nextest run --locked --all-targets --all-features` after this invocation.
- Bounded review: review-inputs provenance gaps at run.md:128 (inv-f5f76bdd incomplete claimed collection; incomplete split bundle); the whole-range review remains outstanding.
- Not inspected: every test file (except a7d1d92e0's wake.rs changes), scripts/wake-e2e.py (+1069), scripts/wake_e2e_selftest.py, scripts/wake_e2e_queue*.py, scripts/release-qualify, the release tests, the fixtures, and the native.rs and codex/composer.rs test hunks.
- The human-tier ask prompt is verified only by permission-rule unit tests; the one live run of the prose path used `--approve-for-me` (ask bypassed).
- The ht-z41e.43 fix for the lazy-target widening has no measurement on non-Codex harnesses (Claude, agy, opencode, Hermes) with multiple bindings.
- Leak and process checks across tasks were manual (stop-run-processes `--check` exit 4); the process sweep reports 1 unknown and 3 kept paths.

## Deferred OK

- Recurring `missing-evidence` cluster (8 tasks): process-class report evidence gaps, not product behaviour.
- `busy-invite/group` T=0 queue-matrix FAIL: model-timing flake, isolated rerun 3/3 PASS; goes to a flake fixer.
- `config-prose/group` row FAIL: its own verdict passed; the failure is `queue_transport_ok: false` in a native-mode run.
- Persisted-rollout side first seen by a non-SessionStart event reads non-auxiliary: fails closed to native, queue routing only.
- Unbounded `session_contacts` and the clear-all of `roots` at 256: small daemon-lifetime growth, and the degraded path only loses `unreceived` tracking.
- Migration-number wording (NodeDelta 20/22 vs 21/23) and the registry's 110 vs 130 byte budget comment.
- ht-z41e.21 retry-journal auxiliary bit sent to a downgraded daemon: needs a downgrade between record and retry.
- ht-z41e.5 tmux recipients stay native under `--queue-proof`: documented in AGENTS.md and RELEASE.md.
- ht-z41e.24 inline final-retirement SQL in seat_config.rs; ht-z41e.36 redundant seam test; ht-z41e.33 regression-guard-only test.
- ht-z41e.6 spike setup deviations (owned PTY, single trials): disclosed, and the conclusions were rechecked live by ht-z41e.43 and ht-z41e.31.
- The human tier is enforced only by harness ask rules (agy, opencode, Hermes, Pi have none): an accepted limit recorded in TRUST-POLICY, consistent with the cooperative model.
