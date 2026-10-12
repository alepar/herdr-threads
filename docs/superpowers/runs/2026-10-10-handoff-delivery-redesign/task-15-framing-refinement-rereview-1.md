# Framing refinement focused rereview 1

Reviewed the complete proposal SHA256 `e0e9783da75c1afb7ba5fcd878674a7dcfd10627a8759d5c52a7806998dc2c14`, including its selected amendment and supersessions, against the settled bounded design and current D source in `.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.3.2`. This is a design review, not implementation approval or a test result. No source, Cargo, tracker or Herdr actions were performed.

**Spec: Important. Quality: Important.** Four concrete contracts below remain load-bearing. The ordered all-class projection, nonordinary recovery outbox, frozen registered codecs and capped real serializers otherwise give a viable narrow implementation direction.

Point-in-time examined source SHA256s: `src/store/seats.rs` = `aa4bf9d178a700e0ce17a1701345cd36bec21c66c7f243f8d2964ed1cf3fd49d`; `integrations/claude/mod/hooks/register.js` = `604e28d528c01e798476faf3a013f5e360cb3c68e8d1dbbfcc53bd1f57584fcd`; `src/protocol/wire.rs` = `05e3351cfd4507bff5d35a48ab374aa0e50fc46c3c1b1513855b0707c244387b`; `src/protocol/watch.rs` = `95f208303ec9bc56b8082df8f0aad744fc15a4da11dda06dd0ae8b6727d6f773`. The author's changing workspace is not a preservation defect; these hashes identify evidence only.

## 1. Name and bound the canonical Attention relevance producer

The amendment chooses `fingerprint.other_pending > 0`, but the actual new presentation entry point cannot supply that condition: `src/store/seats.rs:4572` calls `mod_seat_view_mode(..., false)`, whose early return at line 4630 sets `other_pending:0` (and other digest counts to zero). This is known unfinished runtime wiring, not a newly discovered source regression. The design must nevertheless select the exact replacement producer before code authorization.

The legacy path below that return derives its condition from `attention::seat_digest`, `wake_warnings(...).count()` and `pending_confirming_invitations(...).count()`. Calling that legacy traversal would invalidate the claim that a control-only quantum performs fixed probes and zero discovery walks. The canonical wake counter itself is a suitable persistent epoch; the computed display version is a different value and must remain separate.

Required disposition: name a bounded canonical condition query/projection with explicit candidate/probe accounting and maintenance authority. Evaluate epoch, relevance and surviving control validity coherently in the deciding allocation/replay/confirmation transaction. Do not rely on the metadata-only zero, client state, or a service view read before the deciding transaction. Preserve origin-warning eligibility independently. If the selected condition differs from legacy relevance, explicitly settle that semantic difference and accepted limit rather than calling it the existing condition.

## 2. Lost confirmation can leave stale queued Attention without a clear

Trace: Watch emits and flushes Attention; the mod queues it; presentation confirmation is lost; canonical relevance clears. Under the amendment, the Attention is unconfirmed, its descriptor is removed, and no clear is allocated because only `last_confirmed_attention_epoch` qualifies. The downstream queued Attention can therefore remain eligible even though canonical work disappeared. This is distinct from claiming that the model consumed the output. Existing clear handling exists specifically to remove unsent queued Attention.

Required disposition: either allow a passive epoch-scoped clear for an allocated/possibly emitted Attention (using bounded attempted state, without marking Attention seen/confirmed), or explicitly accept and document this lost-confirm stale-queue limit. A7's unknown downstream uptake alone does not specify this choice. Keep clear allocation bounded, recovery idempotent, and newer epoch debt untouched.

## 3. Existing JS clear consumer is global; epoch-in-ID is insufficient compatibility framing

Actual `integrations/claude/mod/hooks/register.js:238–247` removes **all** non-inflight queued Attention and resets the entire `S.runAttention` set for any `attention_cleared` line. The new contract requires an old clear never to clear a newer epoch. Merely changing IDs does not implement that behavior. Both historical V1 and proposed V2 already use `attention:<decimal>` / `attention_cleared:<decimal>` shapes, so the current proposed ID alone does not distinguish epoch-scoped V2 semantics from historical version-scoped/global behavior.

Required disposition: select an explicit additive V2 marker/optional canonical epoch field or an equally concrete negotiated consumer-mode seam; specify exact old-field omission and V1 behavior. For V2, remove only matching non-inflight queued Attention and only its dedupe/run bookkeeping; retain newer and in-flight entries. Use exact canonical decimal strings, not JavaScript Number. Add actual consumer tests for old-clear/new-Attention ordering, replay, legacy input and notice ingestion. The selected Notice kind, passive/no-ACK/no-submit semantics and ordered shared Watch projection are otherwise concrete.

## 4. Reconcile all serializer work after adding control/clear/replay paths

Proposal line 96 still states `149 + 200 = 349` bounded serializer attempts and `<23MiB` counted output, based on at most one Attention block. The amendment adds clear descriptors, control-only quanta, final native/Watch verification, emitted-control metadata and RPC precommit serialization. It does not expressly state whether Attention and clear can coexist in fresh selection or identify all final/replay passes outside that original ceiling.

Required disposition: publish one explicit per-fresh, per-control-only and per-replay attempt/byte bound. Select at most one control candidate per quantum if that is intended, including clear priority/coalescing, and charge every actual final pass separately. Keep the RPC capped pass separate at its stated <=1MiB scale; do not silently include it in the 65537-byte accounting. Enforce counters or prove traversal structure cannot retry candidate measurements beyond the ceiling. No need to enlarge SQL budgets or rewrite ordinary discovery to resolve this arithmetic.

## Contracts that are sufficiently concrete

- Required ordered `PresentationItemV2` is the sole output ordering source, with <=32 total output items and actual shared `WatchLine + LF` projection. Frozen Notice descriptors repair allocation-frontier/lost-reply loss without inventing receipt authority.
- One bounded nonordinary outbox covers Invitation/Notice/Attention/Cleared, while ordinary messages retain existing ledger semantics. Frozen codec/prefix replay and explicit unsupported/drifted refusal/repair are accepted parent limits; document them in TRUST in the implementation commit.
- `Page.attention_version` is metadata, never evidence of emitted Attention. Only actual surviving emitted stamps can update local seen state or authoritative token transitions. Empty obsolete recovery cannot mark current work seen.
- Five-way first-FULL rotation plus at most one control-only intervention between fresh opportunities can preserve the stated five-fresh/ten-allocation progress bound, once the relevance and clear choices above are fixed. Continuously changing epochs may coalesce; no transient-control delivery promise is made.
- RPC precommit serialization of the actual CommandResult with a fixed512-byte outer reserve is a viable neutral seam. Actual `wire.rs` validates request IDs as <=128 ASCII graphic bytes, UUID strings as canonical36 bytes and version as u16. Worst string escaping is two bytes per ASCII graphic character, so the proposed reserve is conservative for the current fixed envelope. Mandatory full actual WireResponse tests must prove the claimed <=512 overhead including length prefix; future wire fields must update that proof. The generic1MiB cap remains unchanged.

No CLEAN or implementation authorization is implied. A short new amendment can settle these four choices without broadening H/D/I, wake policy, ordinary message ledgers or unrelated production scope.
