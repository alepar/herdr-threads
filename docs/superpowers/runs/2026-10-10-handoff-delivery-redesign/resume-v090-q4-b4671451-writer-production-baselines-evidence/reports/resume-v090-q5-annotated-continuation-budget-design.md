# Q5: preserve invocation bounds in annotated read continuations

Prospective narrow design, 2026-10-11. Requires the parent's standard spec/quality review before implementation; design roast remains OFF. Q4 reader/query/retirement work is independent. This author performed source and supplied-log inspection only, with no source/test, runtime, Cargo, Git, Herdr or nested-agent actions.

## Cause and intended behavior

The unchanged actual body control requests 2048 bytes for a 65536-byte body. `/private/tmp/ht-C-v090-q4-writer-body-budget6-baseline/run.log:66` records step 7 executing the service continuation with `--max-bytes 274` at prior offset 4298, then InvalidBudget with required minimum 288. This is not evidence that the original 2048-byte budget cannot fit a continued chunk.

`fit_annotated_read` (`src/cli/mod.rs:1489–1532`) remembers the invocation bound for the current output, then reduces the backend bound to reserve Lazy annotation overhead. The daemon correctly emits continuation argv using that effective request bound (`src/store/queries.rs:2725–2752`). Following that argv treats the reduced bound as the next invocation's original, compounding reductions until useful continuation fails. History and Search use the same CLI fitter and likewise build their next argv from their effective page request (`queries.rs:2848–2872,2932–2954`).

Preserve the original user invocation byte bound in successful CLI-rendered continuation argv after an internal refit. Each invocation may independently ask the server for a smaller chunk, but every emitted continuation carries the same original bound. The daemon continues to own content selection, opaque cursor and exact progress. No body trimming, cursor synthesis, bound increase, row-limit change, configuration or wire change is authorized.

## Local typed-result helper

Prefer a small CLI-local typed helper applied inside `fit_annotated_read`. It receives the original invocation bound, the current effective request, and a mutable local result. It can modify only these declared command fields:

| Request/result | Allowed field |
| --- | --- |
| Message / ordinary MessageDetails | `body_next_argv` |
| History / History page | `next_argv` |
| Search / SearchPage | `matches.next_argv` |

System MessageDetails, completed bodies/pages without next argv, and unrelated result kinds remain untouched. No traversal of arbitrary JSON, peer text, summaries or event payloads. The operation changes only the budget option's value (or inserts that option when valid default omission requires it). Preserve every other argv element and its ordering: executable/actor namespace, state directory, host routing, output selection, action, message/thread/search literal, opaque cursor, row limit, filters and candidate bounds. Preserve body data/offset/total/completion, typed next cursor, page items, high waters and stop reason exactly as the latest daemon response supplied them.

Only a result actually obtained under a reduced effective byte bound needs normalization. Do not add unrelated rejection behavior to initial responses that were never refitted. Return the locally normalized typed result through the existing `run_wire_result` return path so its advertised result is the one written; full-body display detection continues to use unchanged body completion/offset/cursor semantics. No new caller authority is captured or transmitted.

The existing `InboxInvocationGuard` (`src/cli/output.rs:36–161`) establishes the same budget principle, but also owns inbox-only actor/seat/default-selector reconstruction and thread-local rendering state. Reusing it directly would broaden unrelated behavior and fail to cover returned typed result consistently. Keep it unchanged. A small pure argv budget primitive may be shared later only if it preserves existing inbox behavior; no guard redesign is required here.

## Safe argv normalization

Service continuation argv is a vector of arguments, not a shell string. Identify the budget **option slot**, not every string equal to `--max-bytes`: a search literal, state-dir value or host value may itself look like an option. Consume positional arguments and known option values using the existing CLI grammar/argument-parsing semantics for these three generated forms; do not invoke a command or reserialize its whole argv. Inspecting the option slot must retain original opaque cursor bytes and all unrelated tokens.

An explicit unique `--max-bytes VALUE` must parse as a valid bound and agree with the effective request used for that response before replacing only VALUE with the original invocation bound. History may legitimately omit default bounds through `page_bounds`; absent byte option is valid only when the effective request uses that default. In that case leave omission if the original is also default, or add the original byte option without touching the cursor or row limit. Body and Search currently always emit their byte option. Do not interpret missing mandatory structure, duplicate budget options, a dangling/non-numeric option or an inconsistent bound as permission to guess. An ambiguous rewrite returns an existing structured invalid-response/StoreCorrupt error before any output or cursor-side display effect. Use a short error string without dumping argv or body; its exact size is recorded with implementation.

Do not require new protocol fields or globally strengthen the published result schema. No new cursor decoding, event provenance or server grammar is needed. If existing parsing cannot safely locate the option while preserving tokens, settle the small token-aware helper before implementation rather than using global string replacement. Preserve normal API errors and restart argv; error-result retry redesign is outside this successful-next-argv repair.

## Fitting algorithm and final bytes

Capture original bound M once, before any refit, and retain the original selected request's message/filter/cursor/offset and page limit. Maintain a separate effective request bound E, initially M. For each successful selected result:

1. If E differs from M, normalize its typed next argv to M before rendering. No mutation of the outgoing effective request or service-selected cursor occurs.
2. Look up Lazy delivery modes for this selected result through the existing canonical optional read extension. Render using the exact existing output selection, actor namespace and escaping; annotate those bytes with the selected Lazy marker rows.
3. Measure the resulting bytes, including the restored argv digits and all Lazy markers, against **M**, never E. If they fit, return that result/mode snapshot for the existing final writer. A refit that happens to select no Lazy rows must still normalize and measure: the current unconditional `modes.is_empty()` early return must not bypass this step after E shrank.
4. Otherwise decrease E with the existing checked/saturating reservation rule and reissue the **same read position and selectors**, changing only its byte bound. Discard the previous selected result/modes; let the daemon choose a new page/chunk. Do not truncate body bytes or rows locally, advance to the previous response's next cursor, reduce the row limit, or raise M.
5. Keep the existing minimum-bound/error behavior and server errors when no smaller legal request can fit. E decreases strictly, so the loop is finite. An explicit no-progress check may reuse existing patterns if needed, but must not invent a new failure when a further legal reduction could select different rows.

Use the same normalized result and mode snapshot for output. Keep the final writer's exact byte guard and successful flush-before-AdvanceCursors order; optional later decorations must not escape that guard or consume reserved annotation bytes silently. This repair does not expand unrelated human-renderer or peer-hint policy. If final rendering discovers additional overhead, fail without partial display/cursor settlement rather than write over M. JSON/no-mode paths that never refit retain their current behavior.

History and Search share the repair, not only Message. History preserves thread, direction and opaque pagination position; Search preserves literal, optional thread, max-candidates/filter semantics and the service cursor's topic/body phase. Row limits never shrink in this fitter and must remain exactly the server-requested original row limits in continuations. Mixed pages can gain or lose Lazy rows after re-selection; mode lookup and size measurement therefore repeat on each newly selected response. Compact/history default omission is a legitimate form, not malformed argv.

## Required verification and limits

- Keep the 2048-byte/65536-byte actual body control unchanged as the causal RED. Follow emitted argv verbatim through completion with multibyte body content; every continuation parses to 2048, every actual annotated output fits 2048, offsets advance contiguously to the full byte length, and service opaque cursor bytes match the response. No raised bound, shorter fixture or fixed retry escape.
- Force History and Search through actual annotation refits and multiple next commands. Verify original byte and row bounds, exact cursor/filter/routing/actor/output preservation and final byte sizes. Include default-bound History omission, mixed ordinary/Lazy pages, and a reselected page with no Lazy rows.
- Exercise human/default text and explicit machine text wherever Lazy annotation applies; JSON and capability-unavailable/no-annotation reads retain their existing unchanged behavior. Cover terminal Message/System/no-next-argv and unrelated command results.
- Test token-aware handling of option-looking search literals and routing values, long routing context, duplicate/dangling/non-numeric/inconsistent budget options, and unexpected result variants. Invalid normalization must emit no body and cause no AdvanceCursors, Lazy completion or ACK.
- Verify a deliberately insufficient original bound still returns InvalidBudget; successful refits count restored argv and markers together. Failed writes/flushes retain the existing no-display side effects. Keep inbox continuation controls unchanged.

The CLI can still legitimately refuse when the user's original bound cannot hold minimum framing plus a server-selected character/row and annotation. This design removes accidental geometric loss of an adequate bound; it does not promise every numeric budget succeeds. No production changes or runtime GREEN are claimed in this document.
