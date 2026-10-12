# Independent replacement fixture rereview — 2026-10-11

Spec: CLEAN

Quality: CLEAN

No concrete findings in the exact replacement-only fixture correction. Runtime/lint success for this successor is not inferred.

## Scope and correspondence

Independent inline scoped review using the super-roast evidence discipline, not a full multi-seat PR roast. Read the author's final `resume-watch-fix-report.md` section, actual `/private/tmp/ht-notice-green-native.log`, exact source delta, and actual D CLI/relevance/control/migration references. No Cargo/tests/plugin/Herdr/subagents or source edits were performed by reviewer.

- Actual D baseline HEAD `ec04361ed0fc7efbfa7118d296621a25cf5a1175`, tree `a9f9cadfd328180904d1ec5998bfdfd8d4dfb41e`.
- Baseline source SHA256 recorded in author mapping: `58bfd423e2d9339eca4a7bc9a66d91264f4b6f70250d7af91d543a661a5894fe`.
- Helper HEAD `77054fdfc41ff4ab9ed83032a1db9057d0b6243c`, tree `40e8643b91381b5af753ff989b265ecdee20642e`, observed clean.
- Reviewed source SHA256 `a7626b7fdf09c2590a1d4e34b4983e89f7f8d87b7777e63b2639fd7fec1ac4e8`.
- Actual D working file was already transferred when inspected: identical reviewed SHA, with only `tests/integration/mod_delivery.rs` modified. Its diff against ec04361e is exactly 60 insertions, zero deletions, all in `activation_v2_watch_replacement_replays_uncommitted_output`.

The helper's production baseline remains older; transfer authority is only this exact test file. The service regressions and all other test functions remain unchanged in this delta. Parent owns successor commit and fresh execution.

Final correspondence observation: actual D is now clean at successor `082385c7a0d2945992f419cb5b0c04a41979e3be`, tree `0f9b0336d03ba0870e9a2c903e4463dcc3e12c48`, with identical reviewed source SHA256 `a7626b7fdf09c2590a1d4e34b4983e89f7f8d87b7777e63b2639fd7fec1ac4e8`. The scoped verdicts cover that exact transferred file. No fresh runtime result was read or inferred.

## Meaningful fixture predicate

Actual native5 log SHA256 `4dff8142828094af600620948f028231f9fb3bdafa9a8a066e2df019bd041c83` shows five executed, four PASS, one replacement FAIL. Replacement observed Notice plus passive invitation plus `attention_cleared` for epoch1, and timed out waiting for another Attention. Thus the historical failure is no longer original canonical retirement: the corrected real first Pull settles the informational Warn, leaving no independent open condition in the old replacement fixture. The four actual successes remain bound to ec04361e, not the new test bytes. Author/parent reports scoped leak zero; reviewer ran no leak scan.

The new fixture produces an independent invitation through the existing native CLI `thread create` then `invite --require-ack`, with a different thread from the original passive invitation. Actual D `src/cli/commands.rs:2222` maps this option to `InvitationConfirmation::RequireAck`; migration41 populates the canonical confirming projection from pending invitation digest rows. Actual D `src/store/presentation_relevance.rs:1208` decides Open from this exact seat's confirming projection independently of programmatic Warn settlement. `Controls::open` retains the allocated active epoch rather than the `Controls::absent` path that withdraws Attention and schedules Clear. No SQL seed, fabricated response, extra repairing CheckIn/hook, production relaxation, or timeout increase was added.

The `confirming` query is stronger than an arbitrary count: it joins the canonical projection to the same invitation ID/seat and requires the exact newly created thread, pending state, and `confirmation='require_ack'`. It must return exactly one before original Watch, after old precommit failure before replacement, and after replacement confirmation/stale replay. The source is therefore still unresolved through the operation being tested and cannot pass by silently accepting it.

The added Warn assertion takes the actual original Pull page's Notice warning ID, then requires its retained projection row to be covered by the shared notice frontier for the seat's exact current live binding generation and execution. The inner warning/current-binding query with `query_row(...).unwrap()` fails on missing rows; the left frontier join produces zero if no matching current-execution frontier exists. `covered=true` therefore meaningfully checks that the genuine Warn already settled correctly before replacement. This preserves the diagnosed production repair instead of keeping Warn erroneously live to obtain replay.

## Preserved test strength and quality

All original observations are retained: elected advertised selection and initial accepted owner lease, old actual predispatch Confirm loss, original token still canonical, distinct real replacement lease, replacement's own lease on its Confirm, exact original token and control subset, Confirmed result, stale-old-lease refusal, and exactly one passive invitation raw line byte-identically replayed. Raw matching filters the original passive thread, so the new timed invitation does not weaken or accidentally satisfy that assertion. Original passive `wakes=false`, pending ordinary receipt, zero mod ACK events, and pending unaccepted passive invitation remain checked. The new timed source remains pending too.

The new producer uses existing isolated Rig CLI/daemon ownership and introduces no new process lifecycle or relay gate. Cleanup and unchanged tests retain the prior mechanisms. The patch is additive within one fixture, with truthful `Outside-adapter: tests/integration/mod_delivery.rs`; no production model-visible strings are added. Author reports standalone formatting/diff-check, not compilation or runtime success.

## Remaining gates

Exact committed transfer correspondence is observed above. Complete the unchanged exact Watch three-case selection plus relay unwind helper in the parent lane and scoped leak check even on failure. Compile/lint/default-feature and wider affected gates remain parent obligations. This review evaluates the current fixture against actual corrected production semantics; it does not treat the historical replacement FAIL as evidence that the new fixture passes or fails.
