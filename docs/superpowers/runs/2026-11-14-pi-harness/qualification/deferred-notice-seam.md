# Deferred canonical notice presentation

Neutral seam for PR1-F1 / ht-jr1g.5. Pi consumption requires separate source review and adapter qualification. This change adds no Pi source, caller authority, receipt provenance, peer wire or native wake policy.

`DeliveryPullV2.defer_notice_settlement` defaults false and is omitted when false. Explicit Watch consumers require `delivery.presentation_deferred_notice_v1` together with the existing Watch, cursor and presentation capabilities. Default Claude/opencode Watch keeps exact line output and flush-before-confirm behavior; Hook retains allocation-time notice settlement.

An opted-in Notice freezes its canonical WarningRef and projection ordinal as `context_key=dn1:<sequence>:<event_seq>:<ordinal>` in the existing private wrapper. Its maximum is 65 ASCII bytes. All Notice tags, including retired descriptors, are checked before effects; malformed/unknown/noncanonical/overflow values reject. Allocation validates the exact prefix without advancing its frontier. Existing admitted owner/lease confirmation filters captures already covered, checks the remaining full WarningRef AND ordinal prefix, and advances the frontier atomically with descriptor retirement. A mismatch returns NotCurrentPage with no control acknowledgment or retirement. Current ownership is required even when every capture was already covered. No deferred-prefix queries run for a legacy page without live captures.

A default pull is Busy while an unretired tagged Notice remains. Execution reset retains the unadvanced frontier, permitting normal fresh reoffer. An old decoder accepts the existing context_key string; old confirmation can retire the outbox, leaving the frontier available to a fresh default pull. The compatibility test executes the exact historical confirmation body from 89788571d4c4e4ce2eb1f99206b2edd3a500b523 (SHA256 9e6a0f7b7e9b5751f2c91c879cff1b4c6213ddcdee41096c25a83eb10e580db8), changing only its function name. Its called helpers are audited in the test: new load validation checks valid tags but does not advance frontiers; ownership, control mutation, masks, retirement and save retain their historical semantics. The test uses a real deciding SQLite transaction, a production-generated canonical virtual warning and the new real default pull; no SQL retirement shortcut or historical daemon build.

The local interface is `watch --harness ID --session SID --manual-presentation` and `watch presentation-confirm --session SID` with one bounded opaque JSON receipt on stdin. The latter reconstructs the current claim, fixes consumer Watch and probes capability again. Success reports only `emitted_page_confirmed`; it never claims complete token retirement. The envelope is one JSON line with schema 1, kind presentation_page, raw WatchLine items, aligned opaque keys and a receipt string. Ordinary records have null keys. Other keys hash length-delimited domain, token and Rust item encoding, so pruning preserves keys and a new token starts a new episode.

An actual emitted nonordinary page parks once without automatic confirmation or repull. Ordinary-only output has no receipt. Empty hidden-Attention pages with more=true continue the existing cancellable 1 ms pull quantum, because relevance may need finite progress without an external event. Hidden controls are absent from the emitted receipt. Whole emitted-page coverage remains the consumer's cooperative declaration: exact current branch plus final context for Notice/Invitation/Attention, or exact owner-scoped locally applied clear for AttentionCleared. Ordinary ACK and invitation acceptance remain separate. Confirm success or ambiguity is followed by the consumer's existing owned replacement; the core does not invent a resume authority or token-retired result.

Model pages remain 32 items / 64 KiB. Transport is 72 KiB, metadata 8 KiB, stdin 4 KiB; private descriptor storage remains 262144 bytes and is measured before commit. Production serializers test the worst metadata with 32 distinct 20-digit epochs and u64::MAX versions: receipt 2802 bytes, escaped receipt 3136, keys 2145, empty-items metadata 5350. A fully packed 65536-byte Watch page produces 70886 bytes including LF, below 73728. Keys and receipts never enter model content. Warning-only pending wake evidence remains above the deferred frontier; native threshold/idle/busy/composer guards are unchanged.

Accepted limit: an unaccepted retained page blocks fresh canonical ordinary allocation. Protected adapter storage is a reserve, never priority promotion; cached predecessors drain in priority/deadline/stable-tie order. This neutral change supplies canonical recoverability, while adapter saturation/reload/partial-context and actual settled-native wake qualification remain separate gates.

| Changed core path | Seam/default behavior | Compatibility evidence |
|---|---|---|
| src/protocol/delivery.rs | Explicit false-omitted Watch-only opt-in | Existing literals false; default Watch JSON omission, Hook opt-in refusal |
| src/protocol/capabilities.rs | Additive local capability | Advertised-list/handler tests, old-daemon early refusal |
| src/store/attention.rs | Factor exact prefix read; existing settle calls preserve writes | Existing notice/check-in and presentation tests |
| src/store/invitation_content.rs | Tagged deferred frontier/atomic confirmation/fence; legacy None behavior retained | Causal frontier/fence tests, partial/all-covered/gap/owner/corruption/reset, actual old mutation/new reoffer |
| src/cli/commands.rs | Explicit manual flag and confirm subcommand | Parser regression, existing Watch/ACK parser tests |
| src/cli/mod.rs | Route new local action through Watch precheck/current selection | Existing Watch entry/ACK tests; no raw RPC authority |
| src/cli/watch.rs | Manual envelope/park/continuation; default flush-confirm preserved | Both default channel harnesses exact bytes/flush, old-daemon refusal, finite hidden scan, Close cancellation |
| src/cli/watch/manual.rs | Bounded packet/opaque receipt/current-claim confirm helper | Actual maximum serializers, u64 round-trip, strict bounded stdin, stable keys, clear-only/ordinary-only, validated replies |
| src/harness/agy/delivery.rs | Compile-consistent explicit false only | Existing adapter output/receipt tests; no behavior change |
| TRUST-POLICY.md | Explicit canonical deferred presentation and accepted backpressure | No provenance/admission/native wake change |

Focused evidence is recorded below when checks complete. No full suite, live harness, credentials, real profile, shared service or peer-wire mutation is part of this seam gate.

## Focused validation checkpoint

Causal RED: before the flag existed, both focused cases rejected its unknown field. After compile-consistent false defaults, both reached the real store and failed: allocation advanced the notice frontier to 1 instead of 0, and a default pull accepted the deferred page. After the canonical fix, both passed. The explicit CLI parser case independently failed before the manual flag/subcommand were implemented.

The focused combined/lib/service gate passed 207/207 tests (173 s build, 11.122 s execution), including canonical deferred prefix/frontier/fence/malformed-retired-tag/partial-covered/owner/execution-reset cases, actual historical old-confirm/new-default reoffer, pending canonical notice preservation, current Watch lease refusal, existing Watch/ACK/output tests, both default channel harnesses' exact output and flush order, protocol capability routes and existing wake tests. The final added mixed-page atomic-refusal regression and tightened missing-new-capability test passed a separate 2/2 delta gate (95 s build, 0.905 s execution). The former uses an actual canonical Notice/Invitation/Attention page and verifies the entire retained state and notice frontier stay unchanged, with no control acknowledgment.

The CLI hidden-Unknown sequence is deliberately injected to prove the watch continuation state machine, not evidence of a real relevance scan or native branch uptake. Same-scope replay and execution-only reset are tested directly; generation changes and native reload/overflow/partial-context uptake are not claimed by those fixtures. The pending canonical warning is an unavailable-member notice: it proves notice-frontier retention, not origin p0 native wake. Actual receipt-overdue-after-settled wake belongs to the subsequent isolated native qualification.

## Final neutral gate

Independent source review approved frozen source commit `5b2713226` with no findings. The subsequent root commit `e45e71aeb` changes design/event-contract documentation only. No source change followed the checkpoint.

| Check | Result | Observed duration |
|---|---|---|
| Focused lib/combined/service selection | 207 passed | 173 s build + 11.122 s tests |
| Final atomicity/capability delta | 2 passed | 95 s build + 0.905 s tests |
| clippy --locked --all-targets --all-features -- -D warnings | PASS, no warnings | 94 s |
| scripts/check-default-features | PASS, exit 0; script suppresses successful cargo output | Not instrumented; completion observed within 92 s of wrapper slot acquisition |
| cargo fmt --all -- --check | PASS | 7.31 s |
| scripts/check-harness-neutral | PASS, 0 violations | Not instrumented |
| scripts/check-wire-compat | PASS, 0 violations or stale entries | Not instrumented |
| Scoped process/temp checks in host and builder | PASS, no owned leftovers | Not instrumented |
| git diff --check | PASS | Not instrumented |

The 94 s clippy result exceeds the 60 s per-change budget. This is a recorded budget miss; the run supplies no baseline attribution or introduced-regression conclusion. The 173 s focused build and later 95 s delta rebuild are concrete run timings, not a generic incremental-build benchmark. Root owns the existing budget issue and later final gates. All builds reused the existing target with concurrency capped at four; no stress, full-suite or live run was added.

Retained private evidence: `.superpowers/sdd/implementation-plan/deferred-seam-gates/` contains causal RED/GREEN notes, focused/final-delta output, required checks, scoped cleanup and the frozen source diff. Frozen diff SHA256: `40362022bf4246d852441a0f5f92de50615de2f1c69de8e02b7fd0899c975626`. Independent review: `.superpowers/sdd/implementation-plan/notice-canonical-source-review.md`. These private logs are not part of the public model-visible contract.
