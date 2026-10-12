# Q5 public continuation tests — REVISE

Read-only scoped test spec/quality review. **REVISE before treating this patch as a causal Q5 baseline.** Three concrete fixture/oracle defects would prevent or obscure the intended result. A fourth coverage issue leaves the no-local-trimming requirement unproved. These findings concern the tests, not evidence of production Q5 failure or approval of the helper implementation. No formal roast or nested review was performed.

Pinned inputs:

- Public test patch `resume-v090-q5-continuation-controls-public.patch`: SHA-256 `2dc53296e45ba1227edba1d42d86262e5c45fefef5c2ff9affe288e3ba4b5b3a`.
- Companion proposed source: SHA-256 `d0f16ea147948561122c9f60ad863b4842829cfa6bb3533357180457a4f4249c`. All proposed line references below use that source.
- Approved design `resume-v090-q5-annotated-continuation-budget-design.md`: SHA-256 `19a6a008fdd9d755869182086e3a68f73fcb33f025747c4197c7fbf637091e80`.
- Current production source, mounted parent `tests/cli/lazy_markers.rs`, migration 0026, actual rusqlite 0.39.0 ToSql implementation and current scratch helper draft. No compilation or runtime was performed.

## Required findings

### Q5-PT1 — unsupported u64 SQL parameter prevents compilation

Proposed `Fixture::new`, lines 59–61, uses `for n in 1..=40_u64` and binds `n` directly through `rusqlite::params!`. Project `Cargo.toml:50` enables rusqlite 0.39.0 with `bundled`; its `types/to_sql.rs:272–274` gates ToSql<u64> on `fallible_uint`. This repeats the earlier fixture parameter type failure, rather than creating a Q5 causal RED.

Use an i64 fixture sequence loop or checked `i64::try_from(n).expect(...)`; values 1 through 40 have an exact SQLite integer representation. Preserve all 40 rows, their ordering and original bounds. Parent must compile the corrected test-only patch before interpreting any behavior.

### Q5-PT2 — calibration mutates immutable recorded delivery modes

`boundary(... lose_lazy=true)`, lines 238–245, executes `UPDATE messages SET delivery_mode='ordinary'` and then changes the chosen final row to Lazy. `migrations/0026_lazy_message_delivery.sql:9–11` defines `lazy_message_mode_immutable` and raises `recorded message mode is immutable` whenever this value changes. The initial fixture has 20 Lazy rows, so the first UPDATE necessarily aborts. Both reselection-to-no-Lazy tests therefore fail before the public invocation.

Construct the final mode distribution at INSERT time. For example, calibrate the selected tail identity against a disposable fixture, then build a fresh fixture with only that identity Lazy and the same 40 IDs, sequences, bodies and canonical query inputs. Recheck the boundary against that final fixture. Do not drop or disable the invariant trigger or turn this into an expected fixture error. The row selection and actual Q5 assertion must still come from real StorePort queries.

### Q5-PT3 — Human continuation token oracle compares different namespace stages

`follow`, lines 389–392, compares emitted argv against `raw_argv` after deleting only max-byte options. The delegated parent StoreClient (`tests/cli/lazy_markers.rs:288–302`) always passes `ReadContext.operation_scope=None`; this new Fixture::raw does the same at lines 78–83. `src/store/queries.rs:251–261` derives Human namespace from that canonical scope and explicitly enters false for None. Thus captured raw History/Search next argv lacks immediate `human`, even during a Human CLI invocation.

`run_cooperative` enters the original actor namespace (`src/cli/mod.rs:3082`); after the store query returns, final `encode_selected`/`selected_result` applies that namespace to its rendering copy (`src/protocol/output.rs:293–320`). The actual printed command therefore includes immediate `human`. A correctly repaired Q5 budget still fails the raw token comparison for the Human search and Human no-Lazy history selectors. The ambient namespace guard in boundary does not fix the recorded result because the store overrides it for its query.

Keep the independent parsed `next_actor == original_actor` assertion. Compare the printed command against the actual expected rendering stage: clone the raw argv and apply the existing public `namespace_argv` under an explicit original-actor guard, then require all non-budget tokens/order equal. Alternatively make both real query entry points carry a valid matching operator read scope, if the fixture deliberately models that path. Do not remove the actor oracle or accept arbitrary token differences. This distinction is unrelated to Q5's byte-budget normalization.

### Q5-PT4 — 40-row completion counts selected responses, not printed rows

`follow`, lines 333–356, inserts IDs from the last recorded raw StorePort result and eventually checks seen.len()==40. It parses stdout only for the next command and checks its byte length. A faulty CLI implementation could discard one or more result rows locally while retaining the correct next argv; all selected raw IDs would still be counted, so this test could pass despite violating Q5's explicit no-row-trimming rule. The current assertions are strong for server selection/progress, but do not prove unchanged emitted rows.

Add a per-page oracle that the actual printed row identities/content correspond exactly to the last selected raw result (with the declared continuation budget normalization, actor spelling and canonical Lazy annotations applied only where expected). At minimum parse stable row identity fields and compare ordered emitted IDs to ordered selected IDs; avoid counting marker IDs as rows. Keep the 40 unique-row completion check and use deterministic expected IDs `q5-msg-1` through `q5-msg-40`. This strengthens the requested unchanged completion oracle without changing the fixture size or hiding a production failure.

## Sound portions to retain

- Real SQLite History/Search queries, actual selected responses and opaque cursors supply the page/refit oracle. Calibration only pads routing bytes and explicitly checks raw output fits while canonical annotations overflow; first real invocation requires multiple read selections. No production normalization helper is imported into the tests.
- Each backend request must equal the original command except its byte bound, retaining row limit, original cursor, filters and Search candidate allowance. Emitted commands are parsed by the public CLI grammar, preserving explicit route paths with spaces and actor identity. The next cursor must match the latest raw daemon response.
- Bounds remain 1400/2048, limit 7 and 40 retained rows; continuation is followed verbatim. Mixed mode pages and a final reselected page without Lazy rows are meaningful cases after the immutable-mode fixture repair.
- Complete database snapshots and recorded command allowlists prohibit canonical display/ACK/completion effects; per-invocation journal count detects mutation queuing. JSON/extension-unavailable cases compare exact bytes to canonical encoding and prohibit metadata lookups. The deliberately insufficient 256-byte original budget requires InvalidBudget and empty stdout.
- The new module is mounted under the existing explicit CLI integration suite rather than creating an unregistered target. It creates no subprocess/model/Herdr/config dependency.

## Calibration and coverage boundaries

The calibration loop currently swallows every raw query error at lines 217–219. Restrict skipping to the expected InvalidBudget case; surface unexpected StoreCorrupt/NotFound/ReadBudgetExhausted or fixture errors with their actual reason. Otherwise the terminal 'could not reach serializer boundary' panic obscures whether the calibration envelope or the fixture/store failed. This is recommended for interpretable causal baselines.

These six tests do not by themselves cover all required Q5 grammar/presentation cases: History default-bound omission, option-looking literals/routing values, duplicate/dangling/non-numeric/inconsistent option refusal, terminal/System/unrelated result variants, or explicit human-rendered tables. Here `human=true` selects the Human actor plus `--machine`, while false uses Auto in a nonterminal fixture; it is not a human-renderer test. Those obligations may be handled by the separate helper/body lanes, but the combined verification must account for them explicitly. `OutputSpec` equality does not compare the ParsedCli presentation enum; do not describe it as exact preservation of an explicit `--machine` selector, which the existing History/Search continuation builder does not retain. The current default Search max-candidates assertion checks the actual supported public default; it does not exercise an arbitrary nondefault public candidate flag.

No execution result is claimed. Preserve the unchanged actual 2048-byte long-body RED, distinguish a corrected public test's genuine reduced-next-budget failure from fixture/compiler/oracle failures, then run the same test-only controls against the production implementation. All findings above should be addressed or assigned before treating this lane as sufficient test evidence.

Only this WS report was written. No candidate/test/source edits, Cargo, runtime SQL, Git/index, Herdr, config or nested-agent actions occurred. Quiescent.


## Narrow author repair follow-up

Inspected `/private/tmp/v090-q5-public-controls.rs`, SHA-256 `67c19dff5fcd3dd8c085afb781658800e89f8fec53e785a56e608fa04c41c157`, against the originally reviewed source. Static disposition: **PT1, PT2 and PT3 addressed in this scratch revision.** It uses i64 sequences; calibrates the desired tail identity then creates a fresh 40-row fixture with final immutable modes at INSERT; and applies `namespace_argv` only to an expected rendering copy under the original actor before comparing non-budget tokens. The raw recorded response stays unchanged. No further definite fixture blocker found in those narrow changes.

PT4's selected-versus-emitted row oracle gap remains in that exact revision; calibration still skips all raw errors. These were sent directly to the author. This follow-up is not a compile/GREEN claim and does not change the original patch's sealed REVISE verdict or claim that a regenerated patch was inspected.


## Public3 fresh review — scoped static PASS

Exact patch `resume-v090-q5-continuation-controls-public3.patch` SHA-256 `f4bc4f815568a25fe34b4fc456a12dd500c47d114f108642c95cf5e861a8e03f`; companion source and `/private/tmp/v090-q5-public-controls.rs` both SHA-256 `55c98385ca459cb71f983630895b57d59b5373a9016d0547171c2d0c20366ec0`. **PASS for this public-test lane's static readiness for parent-owned compilation and causal baseline.** Earlier original-patch REVISE evidence stays intact.

PT1–PT3 repairs remain as reviewed. PT4 now compares ordered actual printed History row IDs/full previews and Search matches JSON IDs/full previews/multiplicity with the last raw canonical selection. It counts emitted IDs and requires exactly all 40 expected IDs once. A separate whole-output equality oracle uses canonical encoding with only the known declared budget suffix restored and actual canonical Lazy markers prefixed; local row/body trimming cannot satisfy it. That oracle's suffix assumption is valid for these explicit nondefault bounds and generated History/Search forms; it is not reused as the production general grammar.

The Search continuation parser now reads `matches.next:`, matching the real generic encoder's nested command labeling (`src/protocol/output.rs:546–585`); History remains `next:`. Search row decoding matches `SearchHit`'s tagged kind/data representation. Fixture bodies are below the 256-character preview cap and contain no controls, so comparing full preview data is appropriate. The actor rendering-copy adjustment preserves the independent parsed actor assertion. Unexpected raw errors now fail calibration explicitly, with only InvalidBudget skipped. The original fixed row/bound, real-store refit, no-effect, JSON/unavailable-mode and insufficient-budget oracles are retained.

No additional required defect found in public3. Test execution is still necessary to prove boundary reachability, actual pre-fix causal RED and post-fix GREEN; this review performs neither. Human-renderer and general malformed-token cases remain separate coverage obligations, as noted above.

## Normalization unit controls — REVISE one terminal fixture assumption

Exact patch `resume-v090-q5-continuation-controls-normalization.patch` SHA-256 `06252dd3325885ad26d8bd521d9a69e335fc93c3f5e630bed0d5a7a9fc0e7bcc`; companion source SHA-256 `0a43ff29ec1ab89700a945bcf48d229f3ec5900a7db936a42b3cbaaa42345fa0`. This lane needs the actual proposed helper/method seam to compile, so it is not by itself a pre-implementation public causal test patch.

**Q5-NT1, required:** `q5_normalizer_terminal_and_never_refit_results_remain_exactly_unchanged` (proposed lines 323–343) requests limit100 for the fixture's 40 rows with DEFAULT_PAGE_BYTES=16384, then immediately asserts `next_argv.is_none()` for both History and Search. Compact History can fit; verbose generic Search cannot. Each Search body includes a 196-byte preview and serialized MessageSummary metadata including native event_author. Conservatively omitting event_author entirely still gives at least396 bytes per one-digit-ID row, 15840 across40; the actually present native event_author alone pushes that beyond16384, before larger two-digit IDs, page framing and per-item detail command lines. The producer correctly paginates under the unchanged default bound. This assertion would therefore fail before exercising terminal normalization and be a fixture RED, not a Q5 defect.

Required fix: follow real server continuations at the unchanged default bound to the actual terminal page, proving all40 rows are reached, then exercise no-next normalization on that real terminal result. Keep the separate E==M malformed-argv noninterference check. Do not manually clear next_argv, shrink the fixture, or treat a nonterminal page as terminal. The author and parent were notified.

Other inspected unit controls are meaningful at source level: real StorePort positive records; a direct token-aware literal/route-data control preserving all non-argv result fields; actual default-bound History omission; post-refit-only malformed argv injected after real reduced queries and refused before the actual writer produces bytes; identical duplicate routes and 1024-byte threshold; a no-Lazy refitted final guard with over-bound/exact-bound/never-refit controls; unrelated request/result noninterference; and actual complete Message no-next behavior. All fixture SQL sequences use i64 and no immutable modes are changed. Malformed calibration distinguishes unexpected errors. The direct option-looking literal test intentionally reuses a real opaque cursor while editing the declared result/request literal and never executes that edited cursor against SQLite; it tests token preservation, not semantic acceptance of a cross-filter cursor.

The eight unit controls do not themselves establish System Message no-op, every malformed Body route, or explicit human-rendered final output. Account for those in other lanes or remaining implementation review. The private final-byte guard unit test proves the writer primitive; public3's reselected-no-Lazy test supplies the flow-level assertion. No runtime/SQL-plan/compile claim, production clearance or permission to broaden the helper follows from these unit-test assessments.

Review remained read-only outside this report: no candidate/source/test edits, Cargo, runtime SQL, Git/index, Herdr or nested agents.


## Normalization3 fresh review — scoped static PASS

Exact patch `resume-v090-q5-continuation-controls-normalization3.patch`: SHA-256 `f089bef01ee5ff2f31c4483deee491c881e3d0335ecac8271ceb5d627f3faae8`; actual full scratch `/private/tmp/v090-q5-normalization-controls.rs`: SHA-256 `ec13f3886fd8a747943b142fd5d7ad482d710da6781d35eac6e2d38135a4a353`. Verified that the patch's added unit file equals the reviewed scratch bytes. **PASS for static test spec/quality of this nine-control revision.** Original normalization-patch REVISE remains in the report as historical evidence; NT1 is addressed here. No new required fixture/spec correction found.

- **NT1 terminal pagination fixed without weakening the bound.** The terminal test follows real StorePort typed opaque cursors at DEFAULT_PAGE_BYTES, keeps limit100 and query selectors, clears only History's initial range once using its continuation, rejects duplicate selected IDs and requires the complete exact40 ID set before asserting absence of next argv. It then invokes normalization with E<M and compares the complete terminal result unchanged. The subsequent independent E==M malformed-argv no-op control remains. This now distinguishes legitimate bounded pagination from a normalizer defect.
- **New malformed route controls exercise the actual fitter/writer.** Invalid flag-looking Search thread data, incomplete cooperative selectors and unregistered cooperative harness join the previous malformed cases. Corruption occurs only in real successful reduced-query results, so the test requires a reduced read to have occurred before interpreting StoreCorrupt. Empty stdout and the read-only command allowlist remain required. The Search-only thread mutation is correctly skipped for History, whose generated command lacks --thread. These are malformed continuation semantics, not authorizations or fixture mutations.
- **The actual partial Body control preserves source-selected content.** The new fixture INSERTs one real 65536-byte UTF-8 body: 10922 repetitions of π💡 (six UTF-8 bytes each), plus four-byte tail. A real Message query at1536 must return an incomplete body at offset0 with actual opaque cursor/argv. The test changes only the expected final generated budget VALUE to2048, invokes the helper, compares complete argv, and compares the full typed result with only its declared next argv removed. Body bytes, offset, total, completion, summary and opaque cursor must stay exact. No body or cursor is synthesized to make the helper succeed. This direct helper control complements the unchanged public full-body continuation test; it does not itself establish end-to-end chunk completion.
- **Earlier unit obligations remain intact.** Default-bound History omission, flag-like positional/routing data, raw result noninterference, identical route duplicates and route-size limits, no-mode final-byte primitive behavior, unrelated result kinds and actual complete Message no-op remain covered. No unsupported u64 SQL parameter, immutable delivery-mode UPDATE or fake terminal page was introduced.

The nine-control suite still needs parent-owned compile/runtime evidence on the exact production composition. It does not independently certify System-message no-op or all human-renderer branches; preserve the previously documented cross-lane coverage obligations. Parent separately reports public6 actual PASS on its candidate; I did not run or independently inspect that execution during this review, and do not use it as proof of these unit controls.

Only this WS report was appended. No source/test/candidate edits, Cargo, runtime SQL, Git/index, Herdr or nested agents. Quiescent.


## Body test printed-continuation correction — scoped static PASS

Exact patch `resume-v090-q5-body-printed-continuation.patch`: SHA-256 `426a4cfc4b454dbfb29c692e6dae0e1db12522a0783aa7d44358c37c432f0b11`; full proposed source `resume-v090-q5-body-printed-continuation.rs`: SHA-256 `29401c48f920c6296971166ef9a25c4722907e1e5ea48ae651c4d8812524313f`. Verified that the exact diff from current candidate `tests/service/q4_body_controls.rs` equals the supplied patch. **PASS for this narrow test correction.** It changes only the first positive body test's continuation-following and output oracles; no production behavior is waived.

### Retained actual failure and precise source cause

I read `/private/tmp/ht-C-v090-q4-q5-reader-reconciliation-normalization36-7242f4fb/run.log`: 36 executed, 35 passed, one failed. The body test still executed `--max-bytes 274` at step7, prior offset4298, receiving InvalidBudget with minimum288 (`run.log:39`). That execution evidence remains a failure; this static review does not replace it with GREEN.

`BodyClient::call_with_output` (`tests/service/q4_body_controls.rs:320–336`) clones MessageDetails into `client.last` immediately after the real service response. The CLI later normalizes its own local result in `fit_annotated_read`; it does not rewrite the client's saved backend observation. The old loop read body_next_argv from that raw saved response and executed it at original line454, bypassing the repaired rendered command. Thus a shrinking raw backend bound is expected internal selection behavior after Q5; following it is the wrong post-fix public-flow oracle. This does not excuse a shrinking *printed* continuation.

The actual compact Body renderer emits an incomplete continuation as `more: ` (`src/protocol/output_compact.rs:956–960`). The patch uses that exact label rather than History's next label or the raw backend argv.

### Preserved and strengthened obligations

- Every invocation and every emitted continuation parses through the public CLI Message route, identifies the exact message and retains max_bytes2048. The next command is taken verbatim from one stdout `more:` line with shlex tokenization. No local cursor is synthesized and no larger bound or shorter body is substituted.
- Each actual stdout chunk is <=2048 bytes, successfully flushed, and valid UTF-8. Its indented body bytes must equal the last real backend-selected body_data and the exact original string slice at body_offset. Accumulation now uses those printed bytes, proves strict positive progress and contiguous UTF-8 boundaries, and still requires exact final65536-byte equality. The fixture has no newline/control escapes, making the indented-line oracle exact for these retained body bytes.
- Backend body_total_bytes, offset, completion and typed opaque cursor remain independent evidence. Printed next cursor must equal the backend cursor and use no explicit offset. All raw argv tokens/order must match after replacing only the one expected byte-budget VALUE. Parsed actor and OutputSpec must remain identical. The known generated fixture grammar makes this small expected-value replacement valid; it is not a generalized production parser.
- A completed body must emit no continuation and retain no backend cursor/argv. Partial outputs still leave the own cursor absent and both saved private consumer rows exact. The unchanged final assertions require exactly one AdvanceCursors, exact original full caller claim and exact displayed message, only the own cursor advanced, own passive/version counters increased once with other state preserved, sibling cursor absent and full sibling consumer unchanged. BodyClient still asserts that every AdvanceCursors occurs only after successful stdout flush.
- The unchanged allowlist still excludes ACK and Lazy completion, and the canonical invitation/receipt-pending assertions remain. The loop bound128, original long UTF-8 source, original2048 output bound, canonical send and all sibling/private oracles are preserved. Existing failed-write/flush and read-only negative tests are untouched.

No required correction found in the proposed test patch. Parent must execute it on the exact Q5 production composition to establish printed-next2048 completion; this review neither claims that runtime result nor reclassifies the original failure as a production PASS. Only this WS report was written. No candidate/test/source edits, Cargo, runtime SQL, Git/index, Herdr or nested agents. Quiescent.
