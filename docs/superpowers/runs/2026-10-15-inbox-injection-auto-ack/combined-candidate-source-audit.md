# Combined candidate source preparation — unverified

Inputs: proof branch `verify/inbox-auto-ack-cross-node` at `79fbbcb3` on p2p snapshot `9c85b90d41a30bc39af62653199bbed0a8f5314c`; incoming feature `super-auto/inbox-injection-auto-ack` at `aea4e6fe`.

No cargo, build, lint, test, hook, daemon or Herdr command was run for this preparation. Root owns gated behavioral verification after the code roast exits. Formatting and git whitespace checks are the only checks here.

## Conflict resolutions

- CHANGELOG: keep p2p clean-start/protocol-9 breaking notice under Unreleased; retain the independently released v1.1.0 notes from incoming main. Neither history is silently dropped.
- specs/INDEX: keep all p2p design rows and the dated inbox injection/automatic receipt run link.
- store/control: retain p2p StoreContext argument to decide_accountable and capture its actor for the native-context adapter; pass StoreContext into complete_hook_context too.
- protocol/lazy_delivery tests: retain the p2p canonical ORDINARY claim with node UUID and host locator, then append incoming additive receipt wire/digest tests; discard incoming obsolete instance-only fixture.
- healthy hook goldens: retain incoming automatic-receipt instruction and removal of unconditional inbox ready commands on verified empty delivery; preserve p2p original offer ordinals (Claude offered_through=3, Codex=5), routing UUID/state-dir/host-endpoint, role/seat and sandbox restrictions. These were structurally merged, not regenerated/blessed. Root must run golden harness verification.

## Relevant clean-merge review

- receipts: the incoming standard-context adapter still called the pre-p2p direct writer signature. Adapted it to the existing p2p shared settle_pending(context, tx, decision, actor, seat, operation, ids, observation, SettleHow::Policy, OnHold::Fail). It now authors signed receipt.settled facts via the canonical shared FactRoute writer. No direct receipt SQL was added. Fail propagates out of the enclosing accountable transaction, rolling back any staged attribution and mixed completion on own-history hold. Existing ack_mod event prefix follows Policy in the canonical materializer. No new SettleHow or protocol bump.
- provenance: actor observation remains local cooperative_hook_context with via=hook_context; transmitted fact uses the existing Policy settlement. Remote attribution remains p2p replicated_from, never fabricated local hook evidence. TRUST-POLICY additions describe the local emitted-context claim and retain peer identity/restore semantics.
- protocol/capabilities: receipt_candidates is additive/default-empty/omitted-when-empty; hook.context_receipts_v1 negotiation gates CLI use. Existing host-bearing CallerClaim and p2p protocol remain intact. Legacy lazy-only default behavior and digest shape stay intact when the field is empty.
- control: validates canonical connected registered top-level binding and existing ordinary receipt obligations before deciding; repeats canonical validation in the adapter. Receipt candidates settle within the same transaction as lazy completion. Hook-pull sources remain refused; existing mod/pull paths stay separate.
- constructors: static scan of Rust src/tests CompleteInboxDelivery literals found no missing additive field. All existing peer-only literals receive empty candidates via the clean merge; SemanticMutation remains unchanged and creates an empty candidate list only when converted into the wire command.
- native hooks/inbox: source collects only complete rendered candidates; capability probing suppresses unsupported receipt injection. Completion occurs after successful output and before cursor advance, preserving reoffer on completion failure. DeliveryCoverage controls actual inbox fallback; invitation acceptance remains explicit. P2p node identity and host routing are retained.
- channel/watch: existing AckModDelivered route still reaches canonical shared settlement. Bounded batches append one inbox continuation for overflow/truncation; only complete delivered items are ACK candidates. No new channel-specific receipt writer.
- baseline clean merges: retain main v1.1.0 version/package metadata and bounded disconnected-subagent presence query, while keeping p2p node/journal schema and tests. These are real combined-source differences, not feature-local changes.

## Verification to run after root gate

Run new standard-hook/other public completion two-node matrix, hold retry and existing fact receipt convergence tests against this exact combined SHA; run real-daemon JSON-readonly/text-inbox automatic receipt test, relevant hook goldens, source unit tests and required lint/default-feature/neutrality checks. Track run-id leaks. This preparation makes no passing claim.

## Sibling regression integration adaptation

After combined candidate 082483bd, root precompile identified four feature-regression API mismatches. In native_hook_auto_ack_preserves_sibling_unread_and_delivery_wake only: descriptor.instance_uuid becomes canonical p2p descriptor.node_uuid; read the actual seat host_id from the fixture database as HostId::from_row; pass that host to wake_candidates and both busy/idle reserve_wake calls. No guessed host constant, fixture behavior change or production change. Parent owns the in-flight compilation; no duplicate build or behavioral verification run by this agent.

Correction after root precompile: HostId::from_row is crate-private. The external regression now uses the existing public test_support::host_id_for_test wrapper on that same actual database host_id; production visibility and fixture host selection stay unchanged. No build/test run by this agent.

## Runtime conflict-fixture correction
Initial focused combined run: 50/51 passed; both JSON golden conflict resolutions accidentally acquired a final newline absent from both parents. Byte inspection confirmed each shared parent ends with `}}` and the comparison exhausted equal lines. Removed only the added final newline from two healthy fixtures, preserving all actual content and ordinals; no bless or production change. Full focused proof rerun follows.

## Final sweep-contract merge
Incoming feature4274f130 contains only final test contracts and design-status artifacts after last source prep. Resolved sole INDEX conflict by retaining all existing peer rows and marking the inbox injection row implemented. Source helper remains unchanged. Recheck new capability probe and handoff fixtures on this candidate.

## Final capability-handler probe integration

Source f5ff6c4a imported probe_hook_context_receipts from the pre-node feature branch. Adapted only that probe to the same fixture composition as presentation_capability_routes_use_configured_domain: NodeUuid identity; seeded journal and attached test host; host bootstrap UPDATE; local seats with home_node/is_local/host_id; observations indexed by actual local host; CallerClaim.host locator. Configured DomainService/ControlService signatures already matched once node identity was canonical. The complete real handler path and every pending/acked/provenance/replay assertion remain unchanged. Root authorized only the capability module for this integration check; no full suite or unrelated behavior run.

Capability module command: env HT_LEAK_RUN_ID=0017a6a6-5e8c-4549-8b3b-7fe0e3fbae7d TMPDIR=/private/tmp/ht-cap-0017a6a6 nice cargo test --locked --all-features --lib protocol::capabilities::tests -- --test-threads=1
Result: all 23 capability module tests passed (0 failed), including stable constants, bare-daemon capability defaults and real handler probe. Cargo compilation took 1m15s; test execution 0.64s. Authoritative escalated scripts/check-no-leaked-processes --run-id 0017a6a6-5e8c-4549-8b3b-7fe0e3fbae7d exited 0: no leaked test temp files; no leaked test processes.
