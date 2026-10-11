# Main + D composition Quality review

Verdict: **Quality PASS** for the reviewed composition delta. No Critical, Important, or Minor composition defect found. This is a task Quality review, not a roast, whole-epic approval, runtime pass, or promotion clearance.

Reviewed immutable Git tree: `524ec3a1f68232565e4c32aeb786c14c76d13064`, obtained with `git show TREE:path` and read-only diffs against Main `026ad3ab444779216748cc5733e2241e2834ebaf`, prior A/B composition `3ad07776`, and D `f7aa1cc2`. Checkout path: `.worktrees/handoff-delivery-redesign/.worktrees/upstream-composition-478ec4a9`. Read the full composition brief and resulting normative TRUST-POLICY. Line references below are in that immutable tree.

## Findings and reviewed seams

No actionable findings. The following checks support the verdict:

- `src/store/service_controls.rs:335`: the effective-required and voluntary-joined returns precede invitation selection and command publication. At line 355, the four-column pending query combines Main's lifetime requirement exclusion with the modern Lazy flag; ordinary lookup retains independent released offers, while required lookup excludes every historical requirement linkage. At line 393 the Lazy fresh-episode guard remains. Neither path mutates the existing invitation's frozen policy, timing or origin.
- `src/store/control.rs:945` and `src/store/seats.rs:4154`: both actual acceptance callers share the pending-first helper. It excludes cancellation/rejection for pending selection, then retains the historical noncancelled fallback for idempotence and terminal diagnostics. The stored-operation digest/replay branch at seats line 4143 still precedes fresh selection. The fresh permit binds the same exact invitation that acceptance selects; the complete Main regression exercises the genuine permit path after newer required acceptance.
- `src/store/service_controls.rs:439`: a successful fresh requirement INSERT calls `record_command_change` exactly once, with its allocated sequence and fresh invitation. Release at line 530 reads the actual attributed event's sequence before publishing the released invitation's command change. `src/store/invitation_content.rs:16` and `:43` retain same-instance/positive-sequence checks, monotone checked epochs/revisions and transactional refusal. No producer resets the preserved old invitation when its successor is attached.
- `src/store/service_controls.rs:114` and `src/store/control.rs:1652`: topic publication occurs only on an actual topic change, with checked revision exhaustion; native publication uses the actual event sequence. Service topic no-ops can still bump preexisting directory/filter revisions, but do not publish invitation context. This is not a new composition regression.
- `src/store/invitation_content.rs:500`: hydration associates required command data through the invitation ID and an effective requirement state. The preserved old ordinary invitation therefore cannot inherit its successor's requirement command. The entire invitation-content file is unchanged from frozen D.
- `src/store/seats.rs:4566`: bounded presentation and legacy seat-view modes coexist with Main's permit fix. Bounded presentation avoids the legacy digest walk; legacy mode retains the confirming-only wake contribution and passive invitation content version.
- `tests/store/control.rs:11478`: the rewritten third-attach assertion checks a real successful fresh invitation and requirement, exact linkage, pending/unaccepted state, episode 2 and independent `(100, 500, 600)` timing. Full old invitation and released requirement snapshots are checked before and after replay/no-op. The local old epoch remains unchanged while shared revision advances exactly once and sequence advances. The exact `+1` assertion matches the single command-context publication in the resulting producer. Fresh row snapshots include the new command epoch, and changed-duration effective-required no-op cannot reset timing.
- The complete Main `managed_required_reattach_preserves_upgraded_ordinary_invitation_and_history` function is byte-identical to Main, including real cooperative permit issuance/native acceptance, actor provenance and unchanged join-interval assertions. Four other listed Main controls are byte-identical; `native_ordinary_offer_after_required_offer_can_be_accepted_after_join` differs only by the necessary `confirmation: None` initializer. Both named D Lazy service-attachment controls are byte-identical to D. No SQL-only substitute or relaxed assertion displaced these controls.
- Protocol capability/command/result changes compose additively: explicit Ensure creation policy and `DirectThreadEnsured` coexist with D V2 Pull/Confirm request/result variants. Both Ensure policy and delivery presentation capabilities are present. Omitted Ensure/Invite policy remains omitted from serialization; historical result selection remains `ThreadCreated`. D's consumer capability and bounded result additions survive the automatic merge. The public Ensure-to-registered-Hook runtime coverage gap remains a C task, not evidence supplied by this review.
- `src/protocol/wire.rs`, `src/client/local.rs`, and `.config/nextest.toml` are byte-identical to prior A/B composition. The 2 MiB response exception remains conditional on an explicit modern automatic policy and matching automatic result family; historical None, unrelated, service/watch and errors retain the ordinary ceiling. Client original/family/envelope correlation and both full-pool automatic-envelope test scheduling declarations remain intact.
- Read-only byte comparisons verified migrations 1–31 against Main (31/31), 1–39 against prior A/B (39/39), and 1–41 against D (41/41). No frozen migration was rewritten or renumbered. Main `Cargo.toml`, `Cargo.lock`, `herdr-plugin.toml`, `CHANGELOG.md`, and `docs/evidence/ht-clj-required-invite-reuse.md` are exact Main bytes, preserving the 0.8.1 metadata and historical evidence.
- The resulting trust policy retains Main's immutable lifetime-linkage/pending-first paragraph alongside modern passive Lazy policy, B's scoped automatic-envelope limits, and D's bounded presentation/cooperative output limits. Composition introduces no new authority, adversarial caller verification, seat movement, acceptance or receipt provenance.

## Evidence limits and remaining gates

This reviewer ran no Cargo, tests, harnesses, helpers, Herdr commands, source edits, Git mutations or subagents. The only write is this review report. Static byte comparisons and source inspection are the evidence for this verdict.

The parent's advertised runtime artifact `/private/tmp/ht-main-D-staged-seams/execution.json` did not exist when checked during review. No composed runtime pass is claimed. Parent must retain actual test names/results and source identity, required lint/default-feature/harness-neutral gates and leak evidence separately. Historical D clearance is not automatically runtime evidence for this tree.

C's public Ensure → actual advertised registered Hook context proof remains required. Combined review/gates, whole-epic readiness and later coder roast remain separate. This report does not close the epic or authorize root promotion by itself.

Review complete; reviewer quiescent.

## Verified parent seam-test evidence (follow-up)

Independently read `/private/tmp/ht-main-D-staged-seams/execution.json`, `run.log`, and `leaks.log` after parent completion. This supersedes the earlier artifact-unavailable statement for this focused run only. Quality PASS remains unchanged; no audit was repeated and this reviewer ran no Cargo or tests.

The execution manifest names reviewed staged tree `524ec3a1f68232565e4c32aeb786c14c76d13064`. Its before/after HEAD, merge HEAD, index SHA-256 and staged-diff SHA-256 match exactly. Expected and enumerated selection contain the same eight exact lib test names, and the actual run log reports each PASS:

- `store::control::tests::same_invitation_requirement_attach_release_replacement_has_distinct_context_epochs`
- `store::control::tests::managed_required_reattach_preserves_upgraded_ordinary_invitation_and_history`
- `store::control::tests::service_requirement_uses_an_independent_episode_without_mutating_native_lazy_invitation`
- `store::control::tests::required_then_independent_ordinary_acceptance_keeps_one_join_interval`
- `store::control::tests::native_ordinary_offer_after_required_offer_can_be_accepted_after_join`
- `store::control::tests::managed_required_upgrade_preserves_deadline_and_demands_native_revision`
- `store::control::tests::managed_required_controls_preserve_left_history_and_independent_ordinary_invite`
- `store::control::tests::managed_release_cancels_only_required_invitation_and_replay_retains_history`

Actual command uses `nice cargo nextest run --locked --all-features --lib` with exact-name selection. Result: **8 passed, 0 failed**, one binary, 4396 unselected tests; command exit 0, measured command wall time 0.547850 seconds, nextest test summary 0.320 seconds. Enumeration/build took 183.802829 seconds; this is the parent's full-crate rebuild after multiple source/version changes, not a one-file incremental-build measurement or a full-suite run.

The UUID-scoped leak check used `250d7450-dda6-44e2-8100-8359d6fa28ba`, exited 0, and its log says `no leaked test processes`.

`/private/tmp/ht-main-D-staged-gates/execution.json` was still absent at this follow-up read. No lint/default-feature/harness-neutral gate result is asserted here. C's Ensure-to-registered-Hook proof, broader combined gates and whole-epic readiness remain outside this verified eight-test result. Follow-up complete; reviewer quiescent.
