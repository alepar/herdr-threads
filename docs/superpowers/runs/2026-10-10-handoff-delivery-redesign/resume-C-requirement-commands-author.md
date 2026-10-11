# C canonical requirement-command coverage author handoff

Status: AUTHORING COMPLETE, QUIESCENT at successor `d4ff458e`. Runtime verification pending in the parent-owned Cargo lane. No source changes outside the admitted test file. Initial identity below is retained as history; the successor section is authoritative for transfer.

- Checkout: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/finish-composed-commands`
- Branch: `helper/handoff-composed-requirement-command`
- Base: `601761fabd0cc2beb74c7ec28d0d3d4b6df8076e`, tree `524ec3a1f68232565e4c32aeb786c14c76d13064`
- Commit: `06f3099bb7af77c6ff23386244022f42fd2c02ef`
- Tree: `82de4cdf54c1949dd9eebc6f959662268c96e7a1`
- Changed path: `tests/service/delivery_pull.rs`, 447 added lines, no deletions
- File SHA256: `d46e92eac4896ec39a83440e05e337c376a9cd4334553deb2eb1c3f17c04c978`
- Exact lib selector: `service::delivery_pull::tests::presentation_v2_service_requirement_attach_release_fresh_replacement_keeps_exact_commands`
- Trailer: `Outside-adapter: tests/service/delivery_pull.rs`
- New production model-visible text: **0 bytes**.

## Approved implementation and independent oracles

The parent approved the initial fixture/API plan and its source-driven correction: a confirmed unchanged ordinary item need not repeat after fresh attachment, so preserve the release-phase ordinary proof, then perform a genuine same-seat CheckIn execution rotation to display both pending invitations together. No synthetic binding or presentation repair is used.

The test uses existing `fixture(0)`, `pull_seat`, `presentation_test_pull`, and `rotate`; the existing reserved-service-author/authority-gate pattern; and actual `DomainService::service_operation` calls. It creates its own managed thread through `EnsureManagedThread`, then executes real timed ordinary ServiceInvite, required attachment, ReleaseRequirement, and fresh required replacement. Reads and obligation assertions are scoped to this new thread; wake work belongs to its checked-in recipient.

Each phase pulls canonical V2 Hook output, asserts full literal topic, goal and decimal deadline, and independently validates actual command bodies/argv using shlex and the public CLI parser. Required commands carry their actual invitation ID, linked requirement ID, and canonical revision. Both command forms are checked together after actual execution rotation. Commands are parsed only, never executed; the old ordinary syntax does not imply that native acceptance can bypass the newer pending requirement.

The first required attachment must reuse the unreferenced ordinary invitation while preserving its original 60-second timing; attachment/release advance its command epoch and shared thread context. Release returns the exact ordinary command and canonical released revision 2. Replacement must create a distinct invitation and requirement, retain the complete old invitation/requirement rows, retain the old command epoch, advance shared context once, and own positive command epoch 1 plus its separately requested 90-second timing.

Each offered page is explicitly confirmed with its actual token and exact current emitted control stamps. The structured reply must be Confirmed with the matching acknowledged controls; canonical consumer last-token/outbox/generation/execution and per-invitation complete-offer state are verified. Full canonical obligation rows remain unchanged around Pull/Confirm. Pending invitation/requirement counts, invited voluntary membership, absence of joined intervals/cancellations, and zero receipts/lazy rows remain independent assertions. Timed/required output wakes and canonical wake work remains positive and unchanged by presentation; Attention is permitted.

## Verification actually performed

- `rustfmt --edition 2024 tests/service/delivery_pull.rs`: exit 0.
- `rustfmt --edition 2024 --check tests/service/delivery_pull.rs`: exit 0, repeated after commit.
- `git diff --check`: exit 0 before commit.
- `git diff HEAD^ HEAD --check`: exit 0 after commit.
- `git status --short`: empty after commit.
- Observed commit/tree/file hash recorded above.

No Cargo, build, test, clippy, harness-neutral, default-feature, or full-suite command was run by this author. No runtime pass claim, behavioral RED, or overall C readiness claim is made. No processes or servers were started; there are no owned test children to stop. No Herdr commands, subagents, real configuration changes, push, stash, or composition/source-owner edits were performed. The initial sandboxed Git staging attempt failed on shared worktree index permissions; approved Git-only escalation staged and committed the admitted file successfully.

## Boundary and parent follow-through

This leaf proves the canonical DomainService service-producer to V2 Hook-render/Pull/Confirm boundary when executed. It does not launch an installed hook, verify native codec/stdout transport, deliver to a live model, execute acceptance, or prove live native composer wake. The separate elected-hook C author owns registered Hook/native output proof. This fixture has no message publication, so zero receipt/lazy rows prove absence of fabricated message obligations, not settlement behavior for a preexisting message; other retained tests cover that behavior.

The parent owns transfer to composition, compile/runtime diagnosis, exact nextest name discovery/execution using its sole warm target/jobs=4 lane, applicable leaks/gates, and independent review. The helper checkout and this report are quiescent at the immutable identities above; no pending writes or commands remain.

## Parent-observed compile correction

Parent reported compilation of composition `8382e46c` failed with six E0277 errors, zero test execution, at `/private/tmp/ht-C-requirement-commands-first/list.stderr`; parent reported run UUID `f6a678e1` leak count zero. The author had incorrectly formatted `ThreadId`, `InvitationId`, and `RequirementId` using Display. No runtime verdict is inferred from that failed build.

The parent authorized exactly three format-expression corrections. Each now uses `.as_str()`; no Debug formatting, production Display implementation, behavioral changes, or weakened oracle was added.

- Successor commit: `d4ff458eb61bbe9695ec544e764c80467b491c18`
- Successor tree: `3c922a61550c439cc09ece5637f9345e389b79af`
- Current file SHA256: `79fee9e5cc54797cb3eb55fbc2f3fd50fb4acb021748234d44f62bf48cabf823`
- Delta from `06f3099b`: only `tests/service/delivery_pull.rs`, 10 additions / 6 deletions after rustfmt.
- Same exact test selector and `Outside-adapter` trailer as above.
- Fresh `rustfmt --edition 2024 --check tests/service/delivery_pull.rs` and `git diff HEAD^ HEAD --check`: exit 0.
- Postcommit `git status --short`: empty.

No Cargo/build/test command was run by this author. Parent owns compiling and executing this successor. QUIESCENT: no pending writes, commands, or owned processes.
