decision: local
triggers: shared-cause, defect-type
summary: local — the two-table storage choice (§4) produces four of the findings, but each has a concrete local fix and a single-table redesign was deliberately discarded for sound reasons, so the findings are swept as three rule-clusters instead
pattern: Four findings come from one decision in §4: opaque entries live in a separate `journal_opaque` table while every chain-defining store path (verify_invariants, hash_at, next_ready_head, replay_candidates, journal_forked/serve_branch, promotion) reads only `journal`. A second group is caused by §1 stating only the 68-byte prefix as the cross-generation contract. The rest are independent.
clusters:
- chain-union: rule: any store query or table that defines, walks or freezes a chain position reads a union of known and opaque rows (one named view or reader), never `journal` alone; every table that holds chain rows gets an opaque-capable counterpart in the same migration and a cross-table position-uniqueness guard; startup/verify tests cover an opaque mid-chain position, an opaque head and an opaque fork branch.
- opaque-apply: rule: one apply-side state machine for an opaque chain position; apply stops without advancing applied_through; blocked_on carries a distinct reason; the parked_unknown analogy is dropped; cross-log stall stated in §2 and §5.
- cross-generation-contract: rule: §1 and the MAGIC doc comment list every constant a future generation must keep (hello envelope stays 1, genesis stays HTJRNL01, SHA-256 and ed25519), each pinned by a test or fixture assertion.
