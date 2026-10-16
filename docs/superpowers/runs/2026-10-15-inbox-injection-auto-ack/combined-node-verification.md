# Final combined-node verification

Exact owned candidate: `22924c794853f366327e3fb9ff954cbfbb2f19dd`, branch `verify/inbox-auto-ack-cross-node`, worktree `.worktrees/inbox-auto-ack-cross-node-proof`. It combines immutable p2p snapshot9c85b90d41a30bc39af62653199bbed0a8f5314c and feature source4274f13009409dbef10d26a7fb32a1dbf92bf666 plus explicit canonical-writer/node-host fixture adaptations. The peer checkout was never modified. This is compatibility proof against the recorded snapshot, not a claim that the unfinished peer epic has landed.

## Executed evidence

- Final focused nextest: **76 passed,0failed**,6992filtered,8.067s. It selects fact_convergence_receipt, facts_receipt, real-daemon text_inbox_auto_ack_reaches_remote_sender_json_read_is_passive, healthy/per-event hook goldens, native_hook tests, hook_inbox units, hook_receipt_completion source cases, capability module, current handoff bootstrap golden and installed Claude hook. Exact command below; raw output: proof `.superpowers/final-proof-sweep.log`; run-id `e16f50a1-dc76-4973-8fd6-b2bbbc918014`, isolated TMPDIR `/private/tmp/ht-autoack-proof-final-e16f50a1`.
- Public completion matrix for text inbox, mod, transcript-confirmed hook-pull and standard hook-context requests: B's actual store mutation authors signed receipt.settled facts; A remains pending before transfer, then converges after real emitted facts are transferred twice. Same/fresh operation replay emits no duplicate settlement. Withheld message remains pending, sender due-count retains only it, and reordered prerequisites park then converge. These are public store-boundary tests, not paid live harnesses or a full channel-service admission e2e.
- Hook-context own-history hold: refused completion leaves pending state and no fact/staged attribution; after hold release a fresh operation settles and replicates once. Canonical adapter uses shared settle_pending with `SettleHow::Policy` and `OnHold::Fail`, in the enclosing accountable transaction.
- A real two-daemon test runs JSON inbox first (no settlement), then actual text inbox render/flush on recipient B, observes sender A acknowledged and one actual settlement fact, then repeated display adds no duplicate.
- Actual native hook process tests on combined source pass, including output EPIPE/retry and sibling unread/wake isolation; source authority/read-only/no-obligation negatives and node/host canonical capability-handler probes pass.
- `scripts/test-claude-mod`: strict plugin validation pass; **154 passed,0failed**,1.35s (includes peer tagged-ID cases).
- `nice cargo clippy --locked --all-targets --all-features -- -D warnings`: pass29.70s; `scripts/check-harness-neutral`:0violations,84allowlisted,51entries; `nice scripts/check-default-features`, `cargo fmt --check`, `git diff --check`: pass.
- Authoritative `scripts/check-no-leaked-processes --run-id e16f50a1-dc76-4973-8fd6-b2bbbc918014`: no leaked test temp files or processes. The prior51-test run and capability probe cleanup also passed.

## Integration adjustments and limits

Manual conflict/clean-merge review is in combined-candidate-source-audit.md. Initial proof compile caught new sibling-test node/host API differences; source-only fixes preserve the test's actual fixture host. New capability probe uses canonical seeded NodeUuid/local-host setup. Initial golden comparison caught only an added trailing newline from manual conflict resolution; removed exactly that byte from each affected fixture, without bless. Final tests above include both healthy goldens and current handoff fixtures.

Detailed hook action provenance stays recipient-local (`cooperative_hook_context`, `via=hook_context`); existing wire `how=policy` carries coarse settlement and retains ack_mod event keys. No new SettleHow variant or protocol bump. The p2p tab was informed of rulings, adapter/API and interim passing51-test result; final exact source/proof/audit handoff was sent to the established p2p thread after the clean scoped audit. Peer owns its ht-5fr.49/final branch integration; this run never edited its checkout.

Exact final Rust command (in proof worktree, with run-id/TMPDIR above):

```sh
nice cargo nextest run --locked --all-features --lib --test combined --test multinode --test hook_entrypoint -E 'test(fact_convergence_receipt) | test(facts_receipt) | test(text_inbox_auto_ack_reaches_remote_sender_json_read_is_passive) | test(golden_surfaces::hook_stdout) | test(native_hook) | test(cli::hook_inbox::tests) | test(lazy_inbox::hook_receipt_completion) | test(protocol::capabilities) | test(golden_surfaces::handoff_bootstrap_plans) | test(installed_claude_hook_checks_in_over_socket_and_never_blocks)'
```
