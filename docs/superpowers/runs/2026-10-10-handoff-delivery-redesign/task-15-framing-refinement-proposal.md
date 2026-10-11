# Task15 exact final-output framing refinement — proposal / HOLD

2026-10-11. Design only. This document refines final sizing within parent-authorized M; it does not authorize source/API changes. Fresh boundedness/interface review and parent approval precede implementation. Existing M history, H/D/I, canonical authority, FULL logical-offer counters, current-state retention and emit+flush/TRUST A7 limits remain unchanged. Only this artifact is written by planner.

## Source and observed gap

Inspected D FULLABS: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.3.2` at current dirty D source. Emit-helper owns its adapter/Hook/AGY six-path production branch separately; none of its in-flight API changes are assumed here. Read root `task-15-bounded-design-dispositions.md`, Task15 brief and execution-seams; final217source/16noticevalidation bound supersedes earlier invented all-source100.

Actual source anchors:

- `src/store/invitation_content.rs::pull`: `header=HEADER.len()+2`; selection tests `text_bytes+block.len()` but increments `block.len()+2`. Attention is inserted before persisted five-way rotation. Standalone is plaintext65536-header; required_render_bytes is block plaintext length. `replay` reconstructs an invitation-only frame.
- `src/delivery/invitation.rs::{block_text,join_blocks,render_key}`: block extraction uses frame; join inserts TWO newline bytes between blocks; render_key hashes prefix only. `FrozenInvitation::watch` reconstructs unindented body and revision/epoch string metadata.
- `src/delivery/frame.rs::frame`: HEADER + two newlines + join(two newlines). Body indentation/line normalization, JSON-quoted labels and all header/marker text affect bytes. Empty frame behavior must be counted or explicitly emit nothing, never assumed interchangeable.
- `src/protocol/watch.rs::{WatchLine,WatchItem,WatchInvitation,WatchMessage,WatchAttention}` and `src/cli/watch.rs::write_line`: serde_json::to_vec(actual WatchLine), then one LF, then write+flush. Invitation ID/schema/kind/thread/name/body/truncated/wakes/decimalrevision/decimalepoch all contribute. Message flags/labels and every control/notice representation count too.
- `src/harness/agy/decode.rs::InjectionStep::render`: System JSON `{"injectSteps":[{"systemMessage":{"systemMessage":"T"}}]}`; User JSON `{"injectSteps":[{"userMessage":"T"}]}`. `delivery.rs::pre_invocation` serializes actual Value via serde_json::to_vec. No LF is appended by Hook emit. Literal outside-quoted-string overhead is54bytes System,34bytes User; these counts require equality fixtures against actual Rust codec, not a Python serializer claim.
- `src/harness/agy/delivery.rs`: confirmable pages use downgrade record.step(now); nonconfirmable pages use User when downgraded, otherwise System. Local state cannot be inferred canonically from claim alone. Lifecycle decode's4096byte/prefix path is separate from this observer delivery hook; do not accidentally charge or inject lifecycle guidance here.
- Current DeliveryPullV2 lacks render selector. service/delivery_presentation uses canonical Hook/Watch admission; store cannot safely infer adapter-local codec choice.

Parent/D evidence: eight actual supported Lazy invitations with topic and goal each1024NUL bytes produce about18KiB plaintext but roughly100KiB Watch JSON. Emit-helper TESTONLY diff `task-15-emit-helper-native-frame-red.diff` prepares actual System and downgraded User extremes via Rig.pre, preserving exact content. Planner ran no Cargo; queued RED/GREEN results are author evidence.

## Select registered pure codec descriptors, not caller-described envelopes

Considered alternative: caller sends a bounded generic JSON-envelope byte/format hint. This can bound serialization, but a free envelope count cannot establish agreement with the actual adapter serializer; a max(System,User) reserve loses exact standalone-fit behavior at the20byte boundary. SELECT the registered-adapter PURE sizing/codec-descriptor contract below. No arbitrary callback/query closure, no native harness branch in core, no caller-supplied prefix/suffix native JSON, no local file read inside canonical allocation, no new authority claim.

Proposed neutral interface on HarnessAdapter (default None):

`fn presentation_codec(&self, selector: u8) -> Option<PresentationCodec>`

`PresentationCodec::JsonStringEnvelope { prefix: &'static str, suffix: &'static str }`

The descriptor is adapter-owned static data. Core generic sizing counts a single JSON string in the registered envelope. Registry validates descriptor arithmetic/size and rejects descriptors whose empty output exceeds65536; accepted selector set is the actual finite registered set, not all256 numbers. The method is pure descriptor lookup, invoked before DB allocation; store receives a validated bounded VALUE `PresentationRenderProfile`, not a callback or trait object. No output/authority actions occur during sizing. The generic enum may grow only through separately reviewed codec needs, not permissive arbitrary serializers.

AGY adapter registers two selectors (adapter-local0 System,1 User) backed by the SAME constants/native encoding definition as InjectionStep. Neutral core never tests those IDs or names. Actual native codec equality tests bind descriptor and real serde output for control bytes, quotes, slashes, CR/LF, Unicode and empty content. Default None means V2 Hook unsupported for that adapter; honest historical fallback remains available. Watch uses shared protocol WatchLine serializer directly, not a harness codec.

### Exact proposed unadvertised V2 fields

`DeliveryPullV2.render: PresentationRenderRequest` REQUIRED (new V2 is not yet public/advertised):

- `Hook { confirmable_selector: u8, passive_selector: u8 }`
- `WatchJsonLinesV1` (no caller-supplied line schema/codec/cap)

Hook request is valid only for Hook consumer; Watch variant only for Watch. Canonical service resolves adapter from the existing binding/registry after unchanged admission; validates both selectors through that adapter; profile to store contains two static descriptors and selector IDs. Unknown/unsupported selectors refuse BEFORE confirmed-token retirement, allocation, cursor/item/outbox changes. Request hint is bounded constant bytes and only chooses a supported representation; it never grants HookPull, native rewrite, registration, receipt or Watch ownership authority.

Before pull, AGY computes both selectors from ONE frozen local downgrade/probation decision: confirmable=the currently selected record.step(now); passive=User if downgraded else System. Existing expiration handling occurs before this snapshot. The final page chooses confirmable iff actual nontruncated offered message IDs are nonempty, otherwise passive. Native adapter follows reply's selected selector for that page; do not re-read downgrade state between request and emit. Open probation only under existing actual confirmable-message rules.

`DeliveryPageV2.render: PresentationRenderSelection` echoes `Hook { selector:u8 }` or `WatchJsonLinesV1`, and bounded `output_bytes:u32` gives expected FINAL output bytes. Output_bytes is not a model-visible sentence and not authority. Client validates measured final output equals this count and<=65536 before emission/confirmation. A mismatch emits nothing, confirms nothing, retains pending page and reports a bounded diagnostic. This check is defense against codec drift, not a substitute for canonical exact fitting.

Render key includes domain `presentation-render-v1`, fixed consumer, both validated Hook selector/descriptors (or Watch schema identifier), exact marker_prefix, and outputlimit65536. Hash with existing deterministic SHA256 machinery. Native selector and framing revision are part of the frozen outbox representation. Size cache separates complete eligibility (standalone PASSIVE Hook codec) from accumulated-page codec, which can switch when a confirmable message is appended.

## Exact final-size definitions and selection

Let J(s) be byte length of the ACTUAL serde_json JSON-string encoding, including two quotes. Count using serde_json::to_writer into a capped counting writer, not a hand-maintained escaping table. Let P(B) be actual delivery::frame for ordered blocks B, not a sum of source body lengths.

Plaintext invariant for n>0:

`len(P(B)) = len(HEADER)+2 + sum(len(block_i)) + 2*(n-1)`.

The block_i are the actual rendered/indented/headered strings. Use join_blocks/frame as the source of truth. No trailing separator is charged unless actually emitted; no missing separator in fit tests. Hook selected-codec size:

`H(B,c) = len(c.prefix) + J(P(B)) + len(c.suffix)`.

For AGY actual shapes H=54+J(P(B)) System or34+J(P(B)) User. Client final native bytes are produced by the actual selected codec; serializer equality is mandatory. Empty output follows no-injection behavior, not a synthetic empty envelope.

Watch size:

`W(L) = sum(len(serde_json::to_vec(actual_line_i))+1)`.

Each actual WatchLine carries schema/id/kind and all actual typed item metadata; +1 is its LF from write_line. Measure the COMPLETE batch of lines corresponding to this allocation, not only invitation body/text or one maximum line. Preserve five-way order. A final native Hook envelope or Watch batch including ALL classes must be<=65536 and<=32 delivery items/control blocks. The generic RPC request/response limit remains1MiB: JSON escaping/copies in DeliveryPageV2 are a different measurement. Validate actual WireResponse also fits existing1MiB; no transport cap raise. If metadata adds RPC bytes, actual RPC fitting rejects/rolls back before allocation rather than clipping fields.

Proposed pure core helpers `presentation_output_size(profile, ordered_output_plan)` and `presentation_output_plan(...)` project the SAME ordered candidates used by final emission. Store handles bounded data only; no native callback. Watch implementation must use the same WatchLine constructors/serialization as cli write_line. Hook uses actual frame + selected registered descriptor. Whenever adding a candidate changes `offered.is_empty()`, remeasure the WHOLE trial page under the corresponding descriptor: adding a confirmable message may change envelope and must not invalidate already selected first-class content. If trial fails, withhold new candidate; never downgrade an already selected FULL item to partial because another class changed codec.

Full invitation eligibility is its actual standalone FINAL output under passive Hook codec or actual standalone WatchInvitation WatchLine, including all labels/revision/epoch/prefix/headers/LF. It is NOT8KiB and not plaintext65536. Ordinary message8KiB body truncation remains unchanged. Unsupported alone/full item remains durable incomplete; at most one partial allocation per key/consumer and automatic full retry under later fitting stable render shape. Required_render_bytes index stores exact standalone final bytes for current renderkey, not source length. Unknown/re-render sizing remains in bounded dirty/revisit work; no entire pending scan. Partial alternative is also measured through exact same final serializer and must itself fit; if not, emit no misleading partial and retain obligation.

### Attention: selected scheduling alternative

Do NOT preinsert mandatory Attention. Place first-priority fitting candidate first with its FULL standalone budget. Then append canonical Attention only if whole final output/item count still fits. Otherwise retain an attention-presentation obligation and truthful continuation; a subsequent bounded allocation may carry attention alone. The obligation belongs to current canonical attention version and disappears passively if condition clears, not on invitation presentation. It does not grant a native wake or create a new warning. Attention's actual text/header/version/prefix/JSON/LF is measured like any other output. No arbitrary bytes are reserved ahead of first priority.

This preserves the five-call promise: every continuously runnable class is first once per five successful fresh allocation quanta and then a standalone-fitting head item is FULL, even during continuous notices/ordinary messages/attention. Queueing/eligibility and output allocation are distinct: discovery guarantees finite candidate exposure; once its fitting obligation reaches its fair class head, subsequent first priority serves it. Replay-only calls/failures are not fresh-allocation rotation quanta. Attention may share a page when it fits; if not, it gets one bounded control-only opportunity BETWEEN fresh quanta without consuming/resetting five-way rotation. Schedule at most one such control quantum per canonical attention version after a fresh quantum deferred it; no endless unchanged Attention flood. Successful output consumes presentation only; canonical warning/native eligibility remains independent.

Alternative considered and rejected: reserve mandatory Attention BEFORE deriving standalone bound. It would narrow the existing promise for content that fits a standalone final page and could turn frequent long-prefix attention into systematic starvation. Parent specifically requires no silent narrowing.

## Frozen outbox, replay and representation changes

Persist selected selector/descriptor identity and frozen renderkey/prefix with invitation outbox. Descriptors<=32, canonical plaintext block storage<=65536 AND actual selected invitation-only final encoding<=65536. The mixed allocation's selected codec and both final-size checks happen BEFORE counters/cursors/outbox commit. Page token scopes allocation; no extra complete counter on replay. Fresh response can contain other classes, but invitation-only replay reuses frozen selected codec even when no message IDs remain; do not recompute passive selector and accidentally change output format.

Revalidation remains<=32 canonical descriptor visits. Drop obsolete descriptors; rebuild surviving frozen blocks/lines through the SAME frozen representation and remeasure final output. Removing blocks decreases these JSON-string/JSON-lines encodings; assert remeasured<=original bound. No new prefix/shape/content or companion ordinary/notice blocks inserted under old token. Obsolete-empty replay retires original token and returns truthful continuation; cannot confirm successor allocation. Confirmation does not need renderhint to change authority: exact binding/consumer/token/LiveWatchlease checks remain.

Incoming different renderkey is deferred until outstanding outbox is confirmed/obsolete. Reply echoes frozen selected selector; client can encode it through registry-supported codec, irrespective of its newer preferred selector. This does not claim downstream injection succeeds: parent selected emission-only/TRUST A7. A truly removed/unsupported codec refuses without confirmation or new logical allocation; operator/plugin recovery is required rather than silently rewriting frozen representation. Fresh reviewer must explicitly acknowledge this frozen-replay-versus-newly-observed-broken-codec tradeoff; if parent requires automatic conversion of an outstanding page instead, that is an additional token-preserving repartition design, not silently included here.

Once outbox clears, changed renderkey triggers existing finite dirty-size walk/revisit activation, no offer-counter reset and no initial/context frontier restart. FULL logical offer<=one per consumer/key, <=two across Hook/Watch; repeated physical output after lost confirmation remains allowed. Bounded output bytes includes prefixes/metadata but does not alter exact canonical topic/goal/history. NUL data is preserved through actual serialization, never stripped to pass bounds.

## Bounded work and data, no hidden loops

Retain invitation inspected100 (40/40/20 lending; cleanup<=10), ordinary oneOrdinaryOnlyquery<=100, notice source<=17 plus<=16 canonical notice validations; explicit fixed probes; replay-only<=32. Exclude old invitation traversal/capture entirely. No4096-page drain for invitation discovery.

Sizing is additional CPU work, not additional canonical candidates: at most100 invitation candidates plus32 carried/new ordinary items plus16 notices plusONE attention block are available per fresh quantum. Candidate complete and partial standalone sizing is at most two bounded scans each, plus at most one trial-page measurement per candidate admitted to selection. Count and expose serializer attempts/bytes separately from SQL visits. Use capped writer limit65537; stop as soon as oversized (no materialized6x-expanded buffer). Plaintext candidate rendering is bounded by canonical topic/goal limits, existing message8KiB+boundedmarkers/metadata and requestprefix4096; actual aggregate plan stores at most32 outputs. Trial remeasurement is <=149 attempts*65537 counted output bytes, plus <=200 invitation standalone scans*65537, a conservative explicit <23MiB counted-output ceiling per fresh quantum, not unbounded work. Enforce fixed attempt counters if selected path could revisit a candidate during lending. The bound needs actual assertions; capped writer failure is oversize, not StoreCorrupt unless serializer disagrees with validated sizes. Working output buffer<=65536, trial counting uses no unbounded JSON allocation; generic RPC serialization remains within1MiB.

Input prefix8args/4096bytes; render request2u8selectors/fixedenum; selector descriptors are bounded registered constants and borrowed validated values. No arbitrary user callback, closure, string envelope or new static model guidance. New token/format metadata stays machine-visible, no repeated prose. Check all class metadata and exact serde conditionals; output projection cannot omit notice/attention to pass bounds.

## Remaining concrete interface checks for focused review

1. Existing V2 Watch reply has separate messages/invitations vectors and text is empty for Watch. Final ordered plan must preserve actual class order AND represent every emitted notice/attention. If current consumer projection lacks a notice/control representation, admit a bounded typed ordered output descriptor (<=32 references to existing typed outputs) and use the real emitter representation; do not settle a notice without output or invent a wake-class substitution. Exact current consumer producer/renderer must be agreed by D before implementation. This is a source-backed interface gap, not permission to leave those classes uncounted.
2. Adapter-owned static descriptor and actual System/User serializer equality; freeze both local selector decisions BEFORE request and selected reply codec BEFORE output, including probation/nonconfirmable behavior and replay. Emit-helper adapter.rs ownership requires serialized shared API agreement before either writer edits it.
3. Explicit acceptance of frozen pending representation after local downgrade changes; no hidden automatic reformat. If not accepted, return for narrow parent ruling before code.
4. Persist deferred Attention obligation without weakening existing canonical open-warning/native work; control-only page confirmation does not acquire invitation ACK authority or consume five-way rotation. Source must expose actual continuation despite same recipient content version.

## Required actual RED -> GREEN and independent checks

- D actual canonical eightLazy1024NULtopic+goal fixture: Watch final serde batch<=65536, every canonical byte/metadata preserved across bounded pages; no native wake.
- Emit-helper actual Rig.pre System and downgraded User codec extremes from frozen test diff; test actual V2 allocation/serializer agreement, not merely reject a handcrafted legacy oversized page. Historical V1 output remains exact.
- ASCII, NUL/control6xescape, quote/backslash, CRLF/newline/Unicode labels/goal/prefix, mixed messages/notices/attention and exact separator boundary. Boundary65535/65536/65537 measured through real serializers.
- First-class standalone FULL fits under exact Watch/System/User shape while attention is pending; all five classes served within five successful fresh allocations under continuous other classes. Confirmable candidate changing selector cannot evict first FULL item.
- Lost reply, replay with changed local preferred selectors/prefix, descriptor removal/update, actual output byte validation, failed emission/confirm and binding/Watchlease replacement. No extra logicalcomplete, no ACK/acceptance/history/timer change.
- Partial→full with changed fitting render shape, already-spent partial allowance, exact size-index/queryplan and stable quiet oversized eligibility; no body8KiB carryover to invitationfull.
- Capability preflight and unadvertised readiness; V1bytes/defaultomission/historicalfallback; RPC1MiB untouched and native/output64KiB distinct; schema/outbox shape storage within current additive40 only if unlanded and parent permits (never editreleased/priorapprovedSQL without ruling).

Author reports actual tests/serializer counters; planner makes no GREEN/code-approval claim. Focused fresh framing/interface/boundedness review plus parent authorization precedes any new sizing API or production change.
