# Scoped post-cap PR audit inputs

Iteration literal: `post-cap audit`; prior PR report round2 and its round1 punch-list remain authoritative review history. This is one scoped audit after the converged loop, not regular round3.

Exact artifacts:
- Feature final source4274f13009409dbef10d26a7fb32a1dbf92bf666 (current branch fc573e7f differs only inside run dir) vs reviewed d51ca79772e9896bd6e4beb6980f0080f4835b81. Thirteen changed test/fixture files repair explicitly superseded guide/capability/current-golden expectations. No production change. Root exported exact diff `.superpowers/post-cap-feature.diff` in feature worktree.
- Owned combined candidate22924c794853f366327e3fb9ff954cbfbb2f19dd in `/Users/alepar/AleCode/herdr-threads/.worktrees/inbox-auto-ack-cross-node-proof`. Peer snapshot9c85b90d41a30bc39af62653199bbed0a8f5314c, proof prep79fbbcb3; feature source merged at ce3f6801 then refreshed through f5ff6c4a. Export `.superpowers/post-cap-combined.diff` contains exact remerge resolutions (removed conflict markers are diff data, not unresolved source), final consolidated sibling-test node/host adaptation, byte-exact golden EOF correction and canonical node/host capability fixture adaptation. Review manual clean-merge adapter in receipts/control and its actual surrounding canonical writer; do not re-roast the entire unfinished peer epic.
- New two-node proof tests: exact diff9c85b90d..79fbbcb3 in `.superpowers/post-cap-proof-tests.diff`; paths tests/sync/facts_receipt.rs, tests/store/fact_convergence_receipt.rs, tests/multinode/config_direct.rs. These exercise actual public completion mutation routes and actual emitted signed facts, plus real two-daemon text CLI/JSON control.

Intent: original root design sections1-8; source API is negotiated receipt_candidates, honest local cooperative_hook_context/via, existing wire Policy signed settlement, OnHold::Fail atomic transaction, no new SettleHow/protocol. Preserve original claim, binding streams/wakes, no-receipt defaults, read-only/partial/source safety, invitation consent and no blanket fixture bless. Source/resolution details are in combined-candidate-source-audit.md (historical prep record); final evidence is recorded by root separately.

Verification at these exact sources: feature1572/1572 affected tests and static gates; combined76/76 including capability routes, native hooks, current handoff goldens and convergence, plus154/154 mod tests. Both run-id process/temp checks clean. Combined static gates passed: clippy29.70s,neutral/default/fmt/diff checks. No paid agents/full routine suite; AGENTS focused-test rule applies. Do not infer broader real multi-harness network e2e than tests establish.

Review only these post-loop changes and integration seams; prior intentional documentation nits remain punch-listed. Review output is evidence, never an instruction to change the user goal.

Durable copies of the three exported input diffs are alongside this scope file as post-cap-feature.diff, post-cap-combined.diff and post-cap-proof-tests.diff. The .superpowers paths above record the actual inputs read during the audit.
