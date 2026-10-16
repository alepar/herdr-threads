# Cross-node receipt audit

Read-only audit of the concurrent cross-node implementation; no cross-node verification claimed yet.

The p2p branch centralizes settlement in `src/store/receipts.rs::settle_pending`, which emits `receipt.settled` and applies through `materialize_settlement`. Explicit ACK, text inbox display, mod delivery and hook-pull policy settlement share that writer. Standard hook completion must enter this writer with honest hook-context provenance; text inbox provenance must not be reused for hooks.

Existing coverage: `tests/store/fact_convergence_receipt.rs` proves explicit ACK on B reaches A, text display skips during own-history hold, and old-history delivery avoids duplicate settlement. `tests/sync/facts_receipt.rs::mod_delivered_and_policy_write_the_ack_mod_key` authors fact payloads directly; it does not exercise actual delivery entry points. Multinode `config_direct.rs` and `config_delivery.rs` explicitly ACK; the JSON inbox helper is read-only.

Required combined-candidate verification: invoke public text-inbox, standard-hook and mod/policy auto-completion paths for requested receipts on node B, replicate to sender A, assert receipt settlement and one settlement fact, plus incomplete/read-only/failed delivery remains pending and replay emits no duplicate. Use an owned combined-candidate worktree, inspect receipt/policy/hook conflicts and relevant clean merges, then run focused convergence and new multinode tests with leak-run tagging and cleanup.
