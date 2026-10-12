# Q4-IQ1: retire an inapplicable private Lazy continuation

Prospective supplement to approved episode design SHA256 `472159f833dfb65ba4b5096b69fc87fe461ec82c63864dc794e9f2eef7d9f1ca`; it does not overwrite or retroactively amend that approval. Parent decides this supplement after fresh scoped review. Design roast remains OFF; counters unchanged. This author read source and the WS `resume-v090-q4-implementation-quality-review.md`; no source/test, runtime, Cargo, Git or Herdr action was performed. Parent identifies the unchanged production candidate as `25d`; no runtime result is claimed here.

## Defect and narrow decision

The Q4 query patch correctly rejects a cursorless reader's episode-N body after Reject and reinvitation into N+1. But a private Lazy tail captured during N keeps `ordinary_done=0`; returning InvalidCursor rolls back its old cursor/partial forever. Private publication refresh does not reset an active scan, and the reset at `invitation_content.rs:2347–2368` requires `ordinary_done=1`. Later addressed work can therefore remain blocked despite canonical publication observation. The review supplies source evidence, not an executed regression result.

Allow the private `ordinary_only` source to retire **one exact, intact, modern Lazy captured body whose current reader scope is no longer applicable**. Retirement is bounded private traversal progress, not settlement. Public continuation remains strict. Do not relax the episode predicate or reset the whole ordinary scan.

## Preconditions and result

Before any retirement, retain both existing layers of validation:

- The presentation owner validates its exact binding/consumer/capture and the assembler state before querying (`invitation_content.rs:2369–2404`): one partial for the captured ID with byte length exactly equal to its offset, or the matching skip-body ID with no partial. Malformed identity/offset, multiple/missing prefixes, foreign claims and inconsistent skip state remain errors.
- The query validates the captured message/body and its exact published addressed Lazy authority (`queries.rs:7135–7189`): instance, ordinary message kind/Lazy mode, recipient seat, message/thread/preparation/manifest agreement, publication decision within the captured cutoff, and exact recipient ordinal within the captured Lazy range. The canonical reading binding must match the seat/instance/cursor fence. A missing message/recipient/manifest, mismatched publication, bad range or unreadable canonical value remains an error; do not convert missing authority to a negative scope result.

Evaluate the approved current **modern scope** for that validated captured message once. This is cursor-or-joined-or-current-direct-episode scope, intentionally not the whole fresh unread predicate: an existing cursor advancing past a message does not by itself invalidate its pinned tail. A valid sibling-display continuation remains valid. Only a successful scope evaluation returning false, after intact authority validation, can enter the new branch. SQL errors or absent/inconsistent required authority do not stand in for false.

The new branch additionally requires `ordinary_only=true` and the exact captured recipient row's `legacy_pending=0`. Read that flag from the existing exact addressed-row validation/probe; no uncounted history query is introduced. **Never retire a top-level legacy-pending tail because modern scope is false:** full unread has an independent `legacy_pending=1 AND top_level` branch. This supplement deliberately leaves legacy-flagged continuations on their existing path, including existing strict refusal where applicable. A future broader retirement rule would need a separate proof that every legacy eligibility branch is excluded; it cannot use modern scope alone.

On qualifying retirement, in the current query state:

1. Set `lazy_after_ordinal` to this validated captured recipient's exact ordinal; keep the Lazy high water and publication cutoff unchanged.
2. Clear only this captured `state.body`, and return its exact message ID through the existing `retired_body` channel.
3. Charge one existing physical candidate visit and exactly three Q4 scope probes once, whether retirement occurs in continuation validation or the scan helper. Avoid counting the same candidate again; encoded-size trials never rerun the scope query.
4. Continue the existing bounded scan/rotation from the advanced ordinal. The page may contain later captured-range items, or may report bounded remaining work. No new scan or capture is performed solely for retirement.

The owner requires `retired_body` to equal its prevalidated old body ID (`invitation_content.rs:2414–2421`). Remove only that matching partial/emitted marker; recompute skip-body from the returned continuation using the existing owner flow. Never delete another accumulated body, reuse its offset for a new ID, or treat the discarded prefix as complete display. If the retirement response disagrees with the old capture or any later transaction step fails, roll back all private progress.

## Frozen capture and side effects

Preserve the frozen scan tuple: `ordinary_version`, `ordinary_scan_passive_generation`, captured publication decision high water, and captured source ordinal high waters. Do not overwrite this tuple with the newest passive generation when retiring the body. Current observed publication/passive state can already be newer. Once bounded traversal of the old capture reaches `ordinary_done=1`, its mismatch with that retained newer signal allows the **next normal quantum** to reset and capture fresh published work through the existing branch. No arbitrary extra hook retry limit is a correctness mechanism: finite old captured work must drain under repeated successful admitted calls, then the new episode body becomes reachable. Continuing arrivals/storage failures retain existing progress limits.

Retirement changes only the private traversal cursor and matching incomplete body bookkeeping. It must not modify canonical Lazy recipient/state/legacy flag, message, invitation, membership, receipt, binding read cursor, logical offer stamps, or ACK/display provenance. It emits no remainder as a completed old-body offer, no acceptance, wake or content-free nudge. Preserve unrelated carry, outbox token/control slots, parked progress and all nonmatching private payloads. Physical confirmation before or after this retirement remains physical bookkeeping only; it does not supply eligibility or full display evidence.

The scope check consumes the already approved three added probes and one of the same 100 physical candidate visits (`lazy_scope_probes <= 3*visited` and `<=300`). The legacy flag comes from an already charged exact row. Due1803, carry579 and reconciliation2560 remain unchanged; there is no invitation event lookup, binding fanout or new persistent state. Use the production shared predicate/validation seam, not a copied test-only approximation. If implementation adds another relation read, count and review it before claiming these bounds.

## Required causal controls

Obtain the actual partial/skip-body state from the real private producer, as in the existing sibling-display fixture at `tests/service/q4_body_controls.rs:530`. Physically confirm its nonordinary token without logical body completion; assert cursorless invited episode N. Perform real Reject, fresh Invite N+1 and fresh addressed Lazy Send. Subsequent normal private pulls must advance past only the old exact ordinal, clear only its matching prefix, drain the frozen capture, and deliver the exact N+1 body while the reader remains cursorless and invited. Assert old remainder is not re-offered as complete and no Lazy display, receipt, read cursor or acceptance is fabricated. Include both partial and skip-body owner forms where the actual producer creates them.

Preserve the active-episode sibling-display continuation test. Add same captured-state public continuation refusal, corrupted/missing/foreign publication authority refusal with rollback, assembler ID/offset mismatch refusal, and a legacy-pending top-level control proving modern-scope false cannot enter the new retirement path. Exercise the captured/new passive-generation mismatch: retirement must not acknowledge away the newer signal. Verify one candidate/+3 probes, continued 100/300 bounds, exact unrelated carry/token/parked/offer-stamp preservation, and atomic rollback after tentative retirement. Tests must expose actual private state transitions and framed output; no manually seeded cursor or whole-scan reset may make the liveness assertion pass.

No production fix, runtime GREEN or permission to widen public continuation semantics is asserted by this supplement.
