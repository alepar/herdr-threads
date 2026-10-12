# v0.9.1 composition family — one light review

Status: **NEEDS_FIX**. This is the single task-family review, not a cumulative branch audit or a new design roast. Parent owns one fix pass and verification; this review requests no re-review loop.

Base: `d978fcc4`; reviewed frozen HEAD `f39da231501cb686e361ff656000e777f984735e`, tree `aea4138134a36486aed22bc98aa31d6844e64ee6`. Actual `git rev-parse HEAD HEAD^{tree}` returned these values. `git merge-base --is-ancestor d978fcc4 f39da231` exited 0; the nonempty combined range has 90 changed paths. Merge `495703bd262843558cedd6b974b0d0b9216c40e6` has parents `9dc3b52fa1265443d6dcf8c5f649cb1369674b61` and Main `20e7ca73e41a9449376169ab6875c0675da02edc`. Read-only `git diff --name-only 495703bd..f39da231 -- src migrations` returned no paths: the later parent changes are tests, not production repairs.

Applied the installed current `super-code/task-reviewer-prompt.md` light-review policy. Read the original root design and its accepted composition refinements, relevant normative TRUST clauses, current Main36 composition design, three supplied author reports and parent validation notes. Those reports were treated as claims. No Cargo, test execution, Herdr/tracker actions, Git mutation, delegation, source editing or final helper-file review occurred. The only review artifacts written are this file and the authorized scratch diff.

## Spec compliance

✅ The declared production composition seams satisfy the selected policy on source inspection:

- Independently compared every canonical Main SQL object from Git to current bytes: Main1–36 are byte-exact, the ten feature bodies are byte-exact to integration9dc at37–46, and the filenames form exactly one contiguous1–46 sequence. The initializer imports genuine canonical36 before the shifted feature suffix, and its audits/rebuild thresholds follow that numbering.
- The actual35 preservation key remains registered. Actual36 preservation extends the same retained ACK-binding/cursor/frontier/Lazy oracle with multiple retained in-turn records and exact catalog/rootpages, failed38 rollback at37, retry and physical reopen. Missing36 table/index and late35→36 index collision controls retain unchanged catalog/version assertions. These are real prefixes built from the corresponding SQL; no initialized-latest relabeling is substituted.
- Direct acceptance's helper is byte-exact to integration9dc. A focused reachability check traced accountable `Accept` through the pre-membership current-episode floor capture to `init_for_direct_accept`; generic Join/nonpair acceptance still reaches `init_for_join`. TRUST explicitly scopes Main's head/seat-pending rule around the direct exception.
- Both new wake coverage sites call `pending_confirming_invitations`. A focused check of that shared producer confirmed its timed predicate is inside the indexed selection before LIMIT. Saturation still refuses to assume complete service; one binding's records never serve its sibling.
- A focused authority check traced `AdvanceCursors` through canonical mapping/binding resolution and its retained shared cursor walk. In-turn records require the canonical binding row's `role='top_level'`, plus `request.in_turn`; they record pending receipt/invitation evidence only. V2 physical confirmation supplies neither these records nor ACK/acceptance. Native dispatch reaches the store recheck after its reservation fence, for the chosen binding.
- Codex imports six declared owned groups. A focused setup caller check traced the declaration through `native_entries` and `plan_codex` to the same stable per-event argv. The new event uses the existing Current/Tool route, exact native event metadata and capability-gated standard-hook completion.
- Main pair test adaptations are grounded in the previously selected Q4 policy and add exact cursor/gap/sibling assertions. The original registered names remain. Generic Join's out-of-order displayed-row control retains its held1/shown0 expectations. The mixed timed/Lazy/historical control uses real CreateThread/Invite/AdvanceCursors producers, exceeds the actual1001-row source window without raising its cap, asserts both aggregate and exact-binding answers, and checks immutable deadlines, still-pending invitations and no fabricated receipts/cursors/acceptance.
- The inherited alleged undeclared Claude SubagentStart tuple is corrected to Bogus; declared-event mismatch controls remain. The previously excluded native-entrypoint key is now selected explicitly, exposing its separate fixture failure below.

❌ The combined candidate's focused acceptance checks do not pass: six actual failures remain, plus one deterministic schema-version fixture mismatch in the changed source outside the selected95. These are Important until corrected and covered by actual passing evidence. No claim that they are inherited, environmental or flaky clears them without appropriate base/repeated evidence.

## Issues

Critical: none found in this bounded pass.

Important:

1. `tests/store/schema.rs:5798` — `user_intent_schema22_fresh_and_v21_upgrade` fails its retained-root comparison. Actual output contains the six newly added feature presentation tables (`invitation_context_latest`, `invitation_presentation_consumers`, `invitation_presentation_items`, `presentation_confirming_invitations`, `presentation_warning_fence`, `presentation_warning_proofs`) absent from the historical snapshot. The composition added only `in_turn_offer` to the exclusion. Account for additive tables separately while keeping historical table rootpages and history assertions exact; do not drop the preservation oracle.
2. `tests/fixtures/golden/help/setup.txt:116` — actual `golden_surfaces::help_texts` fails: expected legacy `hook <harness>` registration wording, actual stable `hook-event <harness> <EVENT>`. Update the isolated golden expectations to the already selected stable entrypoint contract, retaining route/ownership constraints.
3. `tests/fixtures/golden/setup/claude.text:4` — actual `golden_surfaces::setup_reports` fails: expected a random `# herdr-threads-owner:<uuid>` command suffix; actual command omits it. Bring the affected setup goldens into agreement with the authorized stable command and separately retained manifest ownership identity; inspect changed output, not a blind expectation rewrite.
4. `tests/fixtures/golden/setup-status/claude.absent.text:49` — actual `golden_surfaces::setup_status_reports` fails: expected `scope: user` at this position, actual introduces `registration_conflict: none`. Reconcile the affected status goldens with the selected stable registration/status interface without weakening foreign/drift checks.
5. `tests/golden_surfaces.rs:803` — actual `golden_surfaces::hook_stdout_per_event` aborts with `BrokenPipe` while writing the hook's stdin, before recording the event surface. No matching base run or repeated flaky evidence was provided. Repair the targeted synthetic native caller/payload fixture as independently required, retain refusal/success assertions and require the actual focused key to pass. Final helper files were outside this review; the failure is recorded without assigning an unproven cause.
6. `tests/hook_entrypoint.rs:6653` — actual `native_event_entrypoints_dispatch_declared_codecs_and_refuse_invalid_pairs_before_effects` fails its initial SessionStart success assertion: stderr reports no registered agent harness among process ancestors. Thus the selected test never reaches its invalid-pair pre-effect controls, despite the correct Bogus tuple edit. Supply the intended real synthetic caller ancestry through the established test mechanism; preserve declared-event mismatch and pre-effect assertions.
7. `tests/integration/session_bindings_smoke.rs:1870` — the changed config8 post-upgrade pin is36, while the composed initializer's latest is46. A focused source check confirmed this fixture downgrades its private database to genuine28, then runs the current `CARGO_BIN_EXE_herdr-threads` daemon ensure. A successful current daemon migration stamps46. Correct this exact expected stamp and retain the subsequent binding/mail checks. This test was not among the95 selected keys; no runtime pass is claimed for it.

Minor (ledger only; no additional code cycle):

- `[doc-drift] docs/agent-usage.md:159` — newly imported universal accept/join/rejoin prose promises head seeding and seat-wide settlement, including direct opening mail. It omits the selected active direct-invitation exception, whose readers retain independent cursors. Scope the generic prose and state the direct exception when the documentation punch list is addressed.
- `[doc-drift] CHANGELOG.md:9` — the imported pair-opening bullet says joining/rejoining seeds the head and pre-join mail settles once for the seat without qualifying direct acceptance. This describes Main's generic policy but contradicts the retained Q4 exception; qualify it in the documentation punch list.

## Test evidence

Actual corrected per-change gates at `/private/tmp/ht-v091-composition-fixture-gates/execution.json`: `cargo fmt -- --check` exit0 (4.701s); `scripts/check-harness-neutral` exit0 (1.353s); `nice cargo clippy --locked --all-targets --all-features -- -D warnings` exit0 (48.148s); `nice scripts/check-default-features` exit0 (34.138s). These are gate timings, not a controlled incremental-build result. The first ce2 compiler diagnostics and tests-only f39 correction remain parent evidence, not causal RED.

Actual focused epoch `/private/tmp/ht-v091-focused-f39d/execution.json` pins the exact reviewed head/tree. Expected and selected inventories are equal at95 keys. Its recorded command is `nice cargo nextest run --locked --all-features --lib --test combined --test hook_entrypoint --test integration --no-fail-fast --success-output immediate -E <95 exact names>`; the full exact expression is retained in that execution file. `run.log` reports **95 run:89 passed,6 failed,5928 skipped**, runtime **6.821s**, run exit100. Initial listing/compilation224.156s is separate; no incremental performance claim is made. All three new genuine36 controls, the new mixed-invitation control, preserved Q4 reader/writer/origin controls and both adapted pair hooks passed. Passing members do not override the six failed members.

The recorded scoped cleanup is `scripts/check-no-leaked-processes --run-id b1a618ae-7fd5-4474-90fb-18b623629077`, exit0; `leaks.log` says `no leaked test processes`. No reviewer-owned processes were started.

Test changes commands: `git diff --stat d978fcc4..f39da231501cb686e361ff656000e777f984735e -- tests` and `git diff d978fcc4..f39da231 -- tests`; both exited0. The test diff has2934 lines/197011 bytes,47 paths,1674 insertions/134 deletions. Read the combined source diff through the authorized scratch artifact, with large golden JSON rows also compared structurally. No added ignores/skips or deletion of the selected existing test registrations was found. The declared pair adaptations have independent policy/cursor grounding rather than expectation changes solely to match output. Mechanical migration renumbering retains the existing preservation/rollback assertions. The unchanged stamp mismatch and six actual failures above prevent CLEAN.

The full suite, authenticated release wake qualification and finalC seal are outside this light pass and are not missing-test findings. Post-fix gates and actual focused GREEN remain required; parent may execute its one fix pass without requesting another task-family review.

## Return packet

`status`: NEEDS_FIX

`finding`: Important items1–7 above, each anchored to its path/line; actual six failures retained individually.

`minors`: the two `[doc-drift]` entries above.

`base`: d978fcc4; `head`: f39da231501cb686e361ff656000e777f984735e; `tree`: aea4138134a36486aed22bc98aa31d6844e64ee6.
