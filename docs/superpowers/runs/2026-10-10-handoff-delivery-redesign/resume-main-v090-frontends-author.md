# Main v0.9.0 frontend composition author handoff

Status: **SOURCE QUIESCENT** on the six assigned paths below. Shared merge checkout remains uncommitted and unstaged by this author. Parent owns index resolution, commit, Cargo/execution, review and promotion. No Cargo/build/test/JS runtime/Herdr/background process/subagent action was performed. No type-check or runtime pass is claimed.

Checkout: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/upstream-composition-v090`

- HEAD: `f330bdab3abee8535213228e5465b085b65be750` (premerge candidate).
- MERGE_HEAD: `b6dd5dd122772599a02e2cc7f76b20e5492f0af3` (released Main v0.9.0).
- Shared canonical-store/service/ledger/schema ownership stayed with the other authors. This author edited only the six assigned source/test files and this report.
- No new model-visible production prose/string was added: **0 bytes**. Main protocol fields/ACK behavior and existing D presentation output are composed, not replaced with new framing.

## Inputs and plan

Read the actual assigned conflicts and normative Main binding attribution/read-cursor/lazy policy, the whole v0.9.0 quality/concurrency impact report, composition plan and schema impact report. Sent the concrete conflict plan to the parent before source edits; parent approved it. The two contracts are distinct: Main resolves each exact native binding and keeps its own reader/cursor/channel; D requires the actual physically emitted page's explicit token/control confirmation and finite owner recovery. Presentation confirmation cannot become message display/receipt authority.

## Resolutions

- `src/cli/mod.rs`: keep Main's environment-derived caller refusal and ActiveGuard before any new D capability connection. Keep D handoff and topology policy/send-time capability preflight before derivation/allocation/effects. Existing auto-merged Main caller/body display/binding logic and D producer guards are retained.
- `src/cli/watch.rs`: `Probe::Supported` carries both independent `presentation` and `cursors` bits from the one definitive capability call. Open the registration once using a cloned exact registering claim. Modern Watch still needs advertised V2 plus the valid actual accepted lease; Pull and physical Confirm take the exact registering Main claim, preserving session/agent/liveness fields. Historical fallback passes `Some(registering claim)` only when binding cursors are negotiated, otherwise retains the seat read. D physical codec/byte checks, flush-before-Confirm, exact control validation, foreign/refused confirmation handling, continuation Close precedence and truncation-hint timing are unchanged. All stream/handshake/continuation budgets are unchanged by this resolution.
- `integrations/claude/mod/hooks/register.js`: retain D contextVersion, canonical invitation epochs/deduplication, invitation wake field and passive notice handling. Every complete Message/Lazy line is completion-eligible, including `ack_required:false` mail settled by a sibling; this preserves Main's AlreadySettled cursor advancement. A nonwaking `ack_required:false` message is passive, while a promoted `wakes:true` message remains active. The D common passive helper continues to classify every notice, unpromoted lazy and nonwaking invitation passively at every pump/tool-context site. Invitation/Notice/Attention never enter message ACK batches. Existing batching/retry/draft/wake budgets and output framing are unchanged.
- `tests/cli/watch.rs`: fake clients retain both independent capability flags and advertise both when set. Preserve both parent appended test groups. Strengthen two existing D cases to check their cross-contract composition: capable positive advertises both flags and asserts the exact Pull/Confirm claim; missing/invalid V2 negotiation falls back to an inbox query with the negotiated registering binding claim. These tests were source-edited only, not executed.
- `tests/integration/mod_delivery.rs`: combine Main's retained custom ServiceConfig fixture/restart support and D's activation-required fixture using private `configured(settings, config, require_activation)`. Ordinary/configured fixtures retain Main config behavior; activation fixtures still require observed advertised presentation. Retain D setup/panic/capability/physical-exchange diagnostics and activation cases, plus Main binding/cursor/promoted-lazy/capacity tests. No fixture timeout increase or retry was introduced by this resolution.
- `tests/integration/channel_activation.rs`: the initializer completion assertion/comment now expects composed latest schema45. Schema owner independently confirmed published Main1..35 plus candidate logical addons32..41→36..45. The library's latest-version constant is `pub(crate)` and unavailable across the integration binary boundary, so the existing literal oracle remains explicit45. Historical inputs, upgrade actions and waits are unchanged.

## Shared interface coordination

Sent exact-claim/reader/AlreadySettled completion requirements to the watch-recovery owner and canonical store-binding owner. Store-binding owner proposed `presentation_seat_view(..., ModBindingRef)` plus ordinary source APIs receiving exact claim or validated BindingId; that agrees with frontend admission and avoids selecting the seat's latest occupant. Parent was copied on the critical dependency. This frontend does not manufacture BindingId authority or repair canonical state; bound presentation slots, due/owner recovery, notice frontier, lease/health and cursor settlement remain their owners' deciding-transaction work. Whole impact's new multi-binding V2 activation/concurrency validation remains required; existing unit mocks cannot establish that contract.

## Historical test dispositions

Static test-name extraction, not nextest enumeration or execution, found:

- `tests/cli/watch.rs`: candidate51, Main45, current54; the complete union of both inputs is present, with no lost or invented test names.
- `tests/integration/mod_delivery.rs`: candidate41, Main37, current47. The automatic merge retained Main's intentional session-binding replacements for three candidate generation-based tests; do not restore their incompatible old assertions:
  - `clear_rebind_stale_generation_ack_refused_and_restreamed` → `clear_rebind_pending_item_restreamed_to_new_session` (same-process /clear supersedes an exact binding, not a seat-generation rotation).
  - `clear_within_rebind_grace_emits_no_session_start_digest_and_no_native_kick` → `clear_new_binding_gets_no_digest_or_wake_for_mail_already_read` and `clear_new_binding_digests_and_wakes_for_its_unread_mail_without_grace` (successor read position and wake are independent from predecessor channel grace).
  - `resume_reack_settles` → `resume_keeps_binding_and_reack_settles` (same-session resume retains its binding/channel).
- `tests/integration/channel_activation.rs`: both inputs and current retain the same2 cases.

These are source dispositions for inventory refresh, not waiver of prior actual C failures or new runtime evidence. Main's preexisting cap fixture uses its injectable ServiceConfig; global production cap semantics stay with the core owners and existing unit proof. All preexisting D owner/replay/capacity/diagnostic tests remain subject to parent execution on the final composition.

## Static checks and frozen file identities

File-specific direct `rustfmt --edition 2024 --config skip_children=true` completed on the five assigned Rust files. Its subsequent `--check` returned exit0; file-scoped `git diff --check` returned exit0. `skip_children=true` prevents formatting unowned module children. All six assigned files have zero conflict markers. No JS interpreter/parser or build tool was run. Git index entries can remain UU until the parent stages the shared merge; that is not a source marker claim for other authors' paths.

| Assigned path | SHA256 at author quiescence |
| --- | --- |
| `src/cli/mod.rs` | `698e7a71a078d1ca0fd1d8057afca00908f93251c4cbac61d6de9367dbca6309` |
| `src/cli/watch.rs` | `7a73421fbfd790cb1ac16fc3871a51444d57baaeea46209d57bd8275bfd081ea` |
| `integrations/claude/mod/hooks/register.js` | `a6310446b444daeddb191f5475e94a81a9f1d052375ae228469fad2dee8a5c17` |
| `tests/cli/watch.rs` | `c48c9348a03b79c67a80cb2b64879de5fae085b8572c4db3672d4bc920aa887c` |
| `tests/integration/mod_delivery.rs` | `5a44e6522aca815516b139e0e2bb92f7225a165ff95239f3dccbd3891999f556` |
| `tests/integration/channel_activation.rs` | `62561b138885d60b5c2e8aac68fb5c45a5ab8217c3eb57802a2841053c66fe2d` |

Parent must freeze the complete resolved tree, run type/lint/default/neutral gates and exact behavioral selections, refresh inventory and obtain independent whole-composition reviews. Author is quiescent pending specific follow-up; no integration/readiness claim is made.
