# Watch owner replay design: scoped rereview

2026-10-11. **WRD-1 ADDRESSED; WRD-2 ADDRESSED. Spec PASS; Quality PASS for the revised design.** The corrections provide an implementable dataflow without changing the selected architecture or declared work/memory/output envelope. No remaining blocking issue was found in this scoped revision.

## Exact artifacts and scope

Read the revised `docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/resume-watch-owner-replay-design.md` and companion `resume-watch-owner-replay-requirements.json` in the outer handoff worktree. Independently computed SHA-256 values match the requested artifacts:

- Design: `ff7fee56d0a93a0b8658966960561f04ff15b5ed18b5323e62f51d561bbdcf3b`.
- Requirements: `699d5a22610b6f2dd235fac3420db08ac1dc01fa9deaeed99d33aeb9de860a40`.

This is a scoped follow-up to `resume-watch-owner-replay-design-review.md`, whose inspected source baseline was nested task HEAD `7bdb3678f8b5ba10a5b51a7bdb6aed2fa163f7d3`. The separately reported missing notice-clear continuation defect is outside this design review. No implementation or runtime success is inferred from design acceptance.

## WRD-1: recorded count domain

The design and JSON now explicitly accept every recorded count in `1..=255`, reject zero, preserve full stamps and distinguish that storage domain from `Offer::is_due` eligibility. They retain the accepted V1/concurrent count-two-to-three-to-four history rather than introducing a clamp, reset or stronger cross-route serialization.

The required owner-replay regression now uses the actual CIR-1 before-record interleaving, demands immediate count-four recovery with unchanged fields, preserves independent nonordinary delivery and kills count-above-three rejection. This closes both the validation ambiguity and its missing compatibility evidence requirement.

## WRD-2: logical continuation and native outlook

The revision supplies an explicit internal ordinary source/bookkeeping disposition, including empty OwnerReplay quanta and None/outbox/control-only work during active recovery. This is sufficient for service finalization to distinguish physical recovery even when no replay ID was emitted. It preserves recovery context through a final exhausted/pruned call.

The allocator's consumed set is now expressly limited to successfully allocated logical IDs. A physical replay cannot hide an unchanged logical due episode. The full-ledger counterexample is specified as a regression: 255 parked count-three IDs plus one due count-two ID on the final physical page must leave logical continuation true, then allow only the due ID's legitimate logical increment.

Native outlook preservation now means leaving the value found under the post-store ledger lock untouched on recovery work, not restoring a pre-store snapshot or assigning false after filtering replay IDs. The rule covers both initial Boolean values, empty/missing quanta, mixed nonordinary pages, final replay and outbox/control-only active recovery. Deterministic concurrent V1 mutation and direct outlook assertions prevent the live registry's wake suppression from masking bookkeeping corruption. Unrelated no-recovery behavior and legitimate normal/due allocation retain their existing rules.

Independent due-cache render/publication invalidation is explicitly retained; physical attempt outcomes do not become logical deferrals. The requirements JSON matches these distinctions and records the relevant mutants. This closes the dataflow gap without adding a protocol field, another ordinary source slice or a wake source.

## Handoff constraints

The original design gates remain required at implementation: exact current lease/canonical execution admission, full-stamp/incarnation pruning, no progress on refusal/rollback, outbox-first immutable recovery/confirmation, one ordinary mode per call, byte-identical retained normal walk state, real sparse/lazy/promotion/truncation fixtures, finite same-lease continuation, count/history preservation, and measured work/output limits. The immediate real CLI reconnect oracle remains unwaived.

The source author should preserve the distinction between design acceptance and executable proof. Review the resulting exact implementation and focused RED/GREEN evidence before claiming the reconnect regression fixed. This scoped PASS does not clear the separate notice-clear defect, whole-D activation, required lint/neutral checks or merge readiness.

No Cargo, tests, Herdr commands, source edits, processes or subagents were used. Only this rereview artifact was written.
