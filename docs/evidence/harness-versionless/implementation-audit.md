# Harness version independence implementation audit

Documentation snapshot: 2026-10-05, production baseline **`2d09b8de6e71aa2ea6f7f93ad0301b60218bfc5f`**. Task1–3 scoped independent source reviews are cleared. **Parent final focused gates are complete at frozen production source; final broad branch review and actual main merge verification remain pending/parent-owned.** This is a documentation/evidence reconciliation, not independent product validation, a native qualification or a release-completion report.

## Approval and history

User LGTM approved approach 1 on 2026-10-05. Original approval commit **`4ce8f9fc`** remains the approval anchor. SHA-256 of its exact approved spec bytes: `63a4304b6295b2b7c63344ca2245a39219917871b5595cc58ce22aac1c780843`. SHA-256 of its initial plan bytes: `06c59bbc9f0e49fad4cf8df2717baeef8dce0a30f254cbad807444aaad1537f9`. Controller file-map clarification `e6b5a1acd7ca4191605a2f3e63286c4b983fb1dd` preceded Task3; Task4 adds only the narrow actual health/Codex/canary map.

The original [design audit](design-audit.md) is historical and unchanged. The [approved spec](../../superpowers/specs/2026-10-05-harness-versionless-design.md) now appends implementation notes; the [plan](../../superpowers/plans/2026-10-05-harness-versionless.md) retains its original checkpoints. Neither notes nor current file hashes replace the original approval. Parent owns exact final current hashes, frozen artifacts and whole-branch/source/API handoff.

| Source stage | Commit | Independent disposition |
| --- | --- | --- |
| Task1 core contracts/parsers | `76e8586f95605c034da03f649c8ab49ad0b51d44` | Important deadline finding plus minor operational-drift coverage finding |
| Task1 R1 | `3b905ca0f1c5c4b34fa37b4e6a08fa675760f7b8` | Scoped compliant/approved; both findings closed; one log-noise caveat retained |
| Task2 setup/launch | `e84b718c92eb2c751b1c0ebd5f3006db3e7d4969` | Important legacy-manifest size regression |
| Task2 R1 | `15f1872c2ff09d82dbf14d89c435e94f70e328c3` | Scoped PASS/PASS; canonical 1 MiB config / 4 MiB manifest limits preserved |
| Task3 daemon/doctor/storage/policy | `f79e3144e9cec31546477d2652fae5a272a9c779` | Important ordinary-doctor to historical-canary consumer mismatch |
| Task3 R1 | `2d09b8de6e71aa2ea6f7f93ad0301b60218bfc5f` | Same-reviewer compliant/approved; I1 closed; cumulative Task3 cleared, HOLD for final gates |

Full reports/reviews live in `.superpowers/sdd/2026-10-05-harness-versionless/`: `task-1-report.md`, original/R1 review reports, `task-2-report.md`, R1 report and original/R1 reviews, `task-3-report.md`, original review, R1 plan/report/rereview. They are local ignored coordination artifacts, not published evidence attachments. Durable execution evidence is under `target/coordinator/versionless/task1`, `task2`, `task3` and `task3/r1`; parent owns final `target/coordinator/versionless/final` logs/archive.

## Current contract and evidence boundaries

- Selected executable/PATH remains the managed wrapper. Core setup/launch/hooks/doctor/daemon observation executes no version/help/schema diagnostic and never inspects a vendor target as admission fallback. Typed registered handles select strict parsers; actual harness/event/session/source/tool/value/paired-role validation precedes mutation and deadline selection. Legacy eventless registration retains the discriminator path.
- Setup configures hooks, emitting `contract_declared` and null installed metadata; it cannot prove delivery or native support. Launch retains actual configured-hook fingerprints, supported-form and canonical host/seat guards, including Codex resume refusal. No new Codex sandbox socket/network/root allowance. Valid unchanged legacy ownership upgrades/status/unsetup remain safe; foreign, edited, damaged or Prepared legacy allowance records refuse. Existing hook transaction recovery remains distinct.
- Runtime metadata is optional and source-scoped. A successful core input is `ContractValidatedInput`, not exact-runtime/model/native receipt proof. Historical recipes, caches and manifests remain diagnostics. Absence of failures or an old `working` label cannot establish current Working. Rich defaults are `PokeCapabilities::NONE`, including optional Claude 2.1.287 metadata; compact recovery, composer stash and turn-time poke need a separately safe qualifier. Historical richer captures retain their exact declarations.
- Codex rollout `cli_version` is creator metadata only, including startup. Known-resume suppression remains; Codex unknown startup is never held for creator credit. Claude runtime-written transcript metadata remains optional. Neither changes canonical daemon binding, continuity, receipt authority or provenance; no adversarial caller verification was added.
- Task3 original 29-file footprint plus six-file R1 yields **34 unique files**. R1 changes no Rust production, policy, migrations or wire. The six seams are admission evaluator, live/saved shell consumers, schema description, Python admission/tier0 guards and Rust doctor consumer integration; exact paths are in the plan clarification.

## Persistence and canary rulings

Exactly one main append: **migration 24**, `0024_harness_contract_diagnostics.sql`; migrations 1–23 remain immutable. Existing nullable `HarnessEvidence` and existing report/Health fields carry diagnostics; no new wire shape/capability/command, fake version/build key or adapter domain/origin is introduced. Doctor's local `contract_declared` label is not a daemon wire enum. Adapter absorption/renumbering of its provisional 24 to 25 is later coordinator work; no adapter or Hermes bridge migration is enabled here.

Unavailable-runtime violations persist under `(harness, session_id, contract_id)` with sticky first event/field/time and monotonic last violation time. Success neither clears nor verifies. Storage: 256 rows/harness; read: 20/harness; retention: 30 days since last violation; deterministic eviction by descending last/first time then ascending session/contract. Health projects one actual failure/harness within 24 hours; report uses existing unattributed reason/time. Missing/empty session and malformed input remain bounded parse/reason diagnostics. Expiry/eviction can retire history; no new reset command. Task3's normative A4 and accepted limits were updated in the same `f79e3144` source commit, with no authority weakening identified by scoped review.

Canary live `chk_t0_admission` and saved `--check-admission` use `evaluate-core`: only `contract_declared` passes, explicitly without exact-runtime/native proof; missing executable is infra; malformed/missing/unknown/historical labels fail. `evaluate-historical` and legacy `evaluate` retain explicit ladder/schema diagnostics and reject core declaration even under equal/unasserted expectation. Historical expectation generation, probe fields/schema shapes and gated Rust captures remain separate and unchanged in behavior. This fixes the producer/consumer seam without adding routine probes or claiming native qualification.

## Recorded focused verification and limits

These are complete summaries reported and assessed in the scoped rereviews, not commands rerun by Task4. Counts overlap across selections and must not be summed as a suite total.

| Stage | Complete selected results |
| --- | --- |
| Task1 R1 | Versionless: 6 library + 3 entrypoint; Codex: 43 pass/2 ignored; Claude: 28; drift: 18; filtered hook unit: 74; entrypoint: 40 pass/11 ignored |
| Task2 R1 | Launch: 32; setup unit: 11; setup library: 81; setup CLI: 46 (170 disjoint final guards) |
| Task3 original | Final regressions: 13; poke: 3; evidence: 40; Health: 32; state: 13; doctor: 31; hook evidence: 33; doctor CLI: 9; wrapper seam: 3; historical integration: 13; wire: 14; schema: 115; attribution: 32; reobserver: 3 |
| Task3 R1 | Final real doctor/saved seam: 6; admission unit/CLI: 21; live shell guard: 13; probe contract: 8; historical expectations: 3; offline selftest: one test over 11 planted cases; historical Rust sample shape: 1 |

Task1's excluded process-exit library test is not passed; built-binary guards cover its process boundary. Ignored native/release/stress cases are not passed. Earlier zero-selected, watchdog-truncated, compiler-only RED or superseded assertion failures are not final GREEN. Task1 minor R1-N1 remains: an interleaved `cannot write meta: No such file or directory` manifest diagnostic has no confirmed producer attribution; it is nonblocking and does not establish a new fixture leak or pristine output.

Original Task3 historical integration can return early without python3/curl: its complete cargo pass does not independently prove every gated body executed. Original logs' source-at-run/exit provenance and separate RED for every later bound/schema guard remain limited; DDL counts (64 tables/132 indexes/156 triggers) are recorded worker audit claims, not a Task4 replay. R1 records commands, exits, source snapshots/hashes and run-id; final seam/admission snapshots match the committed patch, while earlier live guards bind to unchanged relevant paths. Offline/selftest/sample guards are not native capture execution. No native/model delivery, real managed-company wrapper, full-suite or downgrade-binary trial is claimed.

Parent's final handoff reports the following complete gates at frozen production **`2d09b8de`**, with only Task4 documentation dirty. These are parent-supplied results, not Task4 execution or independent log verification:

| Final gate | Reported result |
| --- | --- |
| All-target/all-feature clippy | exit 0; 27 s |
| Default-feature gate | exit 0; 18 s |
| Versionless selection | 21 library + 5 combined + 3 entrypoint + 1 integration passed |
| Filtered hook unit | 74 passed, complete summary |
| Hook entrypoint | 40 passed / 11 existing ignored; 48 s |
| Historical evidence, nocapture | 13 passed; no prerequisite-skip messages |
| Actual doctor/canary seam | 6 passed |
| Formatting / cleanup | fmt exit 0; cleanup run-id prefix `E23303A5`, exit 0; all helpers settled |

Durable final logs: `target/coordinator/versionless/final`. These runs bind to production `2d09b8de`, **not the future Task4 docs commit**; Task4 leaves production identical. Final historical nocapture results address the earlier prerequisite-execution visibility concern for this rerun without rewriting original evidence. The earlier manifest-write diagnostic did not recur in final hook-entrypoint/filtered regression logs; its original producer remains unconfirmed and nonblocking.

Final source-at-run evidence, broad review, archive/hashes, integrated sweep and actual main merge verification remain with parent/coordinator. Coordinator w4:p1 owns release 0.2.8 and finished-topology cleanup. Installer joins after versionless; adapters absorb later. Overall DONE+MERGED requires independent verification of the actual landing. No full suite or native qualification is claimed by these focused gates.

Task4 ran no cargo, tests, compiler, canary, test helpers, servers or process cleanup; it started no persistent processes and touched no real configs, foreign branches, product source, tests or migrations. Its validation is documentation diff/footprint/history preservation only.
