decision: local
triggers: shared-cause, defect-type
summary: local — five independent round-1 findings with distinct local fixes; the delete-on-promote schema surface frozen for a promotion deferred to ht-evlm is closed by removing that surface before tagging
pattern: Two findings share the §4 CHAIN_ROWS union decision (salvage on un-migrated backup; verify_invariants seek); two share the §3 park-on-empty-cut gate (missing push_link; withheld counts logs); one is the promotion surface left after deferral.
clusters:
- promotion-surface: r1 [Should-fix] src/store/journal.rs:1962; migrations/node/0004_journal_opaque.sql:19; migrations/node/0004_journal_opaque.sql:21 | rule: nothing in the unreleased 0004 delta or in src may encode a promotion storage model while ht-evlm owns that decision; remove journal_opaque_promoting, the delete exemption, promote_opaque, apply::unpark, their tests, their entries in verify_invariants and src/service/kicks.rs, and the stale header; keep plain append-only and immutability guards.
