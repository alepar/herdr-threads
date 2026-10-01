| id | location | finding (short) | disposition | evidence / reason |
|---|---|---|---|---|
| P1 | src/harness/setup.rs:620 | `setup codex` printed line always prepends `--no-daemon` | ROAST | Still `vec!["--no-daemon"]`. Managed launch dedupes it (harness/launch.rs:335-358). This only bites a user whose shell alias already adds the flag. |
| P2 | src/cli/setup.rs:747 | `codex_layer_paths` walks to `/` when there is no `.git` | ROAST | Unchanged. Fails closed: at worst a spurious exit-1 refusal. |
| P3 | src/cli/setup.rs:851-854 | Duplicate detection is a substring match; the "different hook" heuristic is loose | WAIVE | Unchanged. Only refuses or warns; nothing is written. |
| P4 | docs/install.md:124 | Docs never say session-flag Codex hooks are `untrusted` | FIX-NOW | No trust/`bypass-hook-trust` text in docs/README/src. Every Codex evidence run (codex-158 capture, demo 3, matrix 2 report.md:33,138) passed `--dangerously-bypass-hook-trust`. A user who follows the docs may get hooks that never fire. |
| P5 | cli/setup.rs version probe | `codex --version` creates `$CODEX_HOME/tmp/arg0` | WAIVE | Inherent to running Codex. |
| P6 | harness/codex_schema.rs:418; cli/hook.rs:268-276 | Cold fingerprint over the hook budget never writes the cache | FIX-NOW | Still open. 0.159.2 is admitted only through the schema fingerprint, so every live Codex path depends on this cache. The only ReadWrite writer is the budget-bound hook; doctor is ReadOnly; setup and launch (setup::observe → `InstalledVersion::observe`, codex.rs:622) use `FingerprintCache::Memory`. A slow cold disk means the hook refuses permanently. |
| P7 | codex_schema.rs:357 | Cache read-modify-write has no lock | WAIVE | Lost entry only costs a rescan. |
| P8 | codex_schema.rs:357 | `*.tmp-<uuid>` leak after a watchdog kill | WAIVE | Bounded to rare kills; private dir. |
| P9 | harness/codex.rs:476 | `version_output` timeout not capped | WAIVE | No live effect. |
| P10 | host/native.rs:1051,1167 | Adapter never reports `EmptyShell` | ROAST | Occupancy is still always `Unknown`. This is an adapter capability gap (it also makes W5-4's guard unreachable live). |
| P11 | store/seats.rs planner | Old-planner edge case dropped | WAIVE | Informational; no state reaches it. |
| P12 | multiple tests | Tests flaky under parallel load | WAIVE | CI runs `--test-threads=1` (ci.yml:102). |
| P13 | tests/hook_entrypoint.rs | Exits 101 under the parallel harness | WAIVE | Same as ht-4is.11.8 (waived for this run). |
| P14 | validate-native-demo.py manifest_source | `cooperative` shown with `--cli-hint` | WAIVE | Label only. |
| P15 | validate-native-demo.py:1812 subagent_activity | Nested `claude -p`/`codex exec` via the shell not counted, so child-ACK can PASS | ROAST | Unchanged. Validator false-PASS path, but needs an adversarial model shape. |
| P16 | validate-native-demo.py setup cwd | `setup codex` run from the driver cwd | WAIVE | Harmless today. |
| P17 | test_demo_verify.py:436 | Test injects `child_check` by hand | WAIVE | Cannot produce a false PASS. |
| P18 | validate-native-demo.py manifest | Schema-matched admission not in the manifest | WAIVE | It is recorded in summary.json. |
| P19 | cli/hook.rs encode_native | Overview rows trimmed before commands | ROAST | Explicit `has_more`. Design order question. |
| P20 | cli/hook.rs:548-555 | Final fallback emitted without a fit check | ROAST | Still `return envelope(&compact(&keep))`. Not a regression. Ties to W6-D5 (S15 fails with long state-dir). |
| P21 | protocol/attention.rs | `deny_unknown_fields` breaks a mixed-version upgrade | WAIVE | Single binary; disclosed. |
| P22 | validate-native-demo.py | PreToolUse vs SessionStart mislabel | WAIVE | Cannot fabricate a delivery. |
| P23 | validate-native-demo.py:188 | `auth` regex too broad | WAIVE | Never produces a PASS. |
| P24 | validate-native-demo.py auto relaunch | Stale binding after an auto retry | WAIVE | Diagnostics noise only. |
| P25 | .github/workflows/ci.yml:171 | Clippy runs only with `--all-features` | ROAST | Unchanged. Cheap to add a step, but low value. |
| P26 | store/seats.rs | Let-chain indentation | WAIVE | Cosmetic. |
| P27 | cli/setup.rs allow_rule_json | "re-run setup" note is wrong for a rule the user held | WAIVE | Wording only; behaviour correct. |
| P28 | harness/claude.rs unsetup | Empty `hooks:{}` left behind | WAIVE | Cosmetic, pre-existing. |
| P29 | cli/setup.rs re-setup conflict | Moved-binary conflict message misleads | ROAST | UX. |
| P30 | docs/agent-usage.md:95 | Stale version list | FIXED | 19b9f9d; line 95 now says 2.1.283–2.1.286 and 0.159.2 schema-matched. |
| P31 | validate-native-demo.py:217 invokes_herdr_threads | `env`/`sudo`/`exec`/backtick wrappers not recognised | ROAST | Unchanged. A denied wrapped call shows as info, not an S18 FAIL. |
| P32 | validate-native-demo.py:264 agent_shim | A body word `--state-dir` suppresses pinning | WAIVE | Contrived. |
| P33 | harness/mod.rs:196 READY_HEADER | "exactly as written" sits above a placeholder line | WAIVE | Labelled; harmless. |
| P34 | cli/hook.rs:111-124 | Empty `--state-dir=` accepted | WAIVE | Filtered by `is_absolute`, so the hook stays quiet; setup never emits it. |
| P35 | protocol/commands.rs envelope | Request racing a restart reaches the new boot | ROAST | Design: server-side `expected_boot` for non-keyed mutations. |
| P36 | client/mod.rs:19 | `transport_denied` remedy always blames the sandbox | WAIVE | Wording. |
| P37 | tests/daemon/ownership.rs:674 | Test relies on fixture path length (legacy name) | ROAST | Unchanged. Test hygiene; use an unrelated or wrong-boot name. |
| P38 | daemon/ownership.rs:492 | macOS full-backlog ECONNREFUSED note missing | WAIVE | Owner lock is the real guard; doc comment only. |
| P39 | docs/agent-usage.md:30 | Replacement occupant re-offered oldest notices | WAIVE | By design, disclosed. |
| P40 | run.md:232 | Crash fix3 real-host acceptance not run | ROAST | Evidence gap for the final roast. |
| P41 | run.md:228 | 10.3 fix3 parked items not enumerated | ROAST | Enumerate from the comp-package-lifecycle reviews. |
| P42 | run.md:236 | 11.7 parked 0/0/1 not enumerated | ROAST | Same. |
| P43 | run.md:245 | hook+digest seam 3 Nits not enumerated | ROAST | Same. |
| P44 | run.md:245 | Claude 2.1.285 merge 0/2/3 parked | ROAST | Same; 2 Minor unenumerated. |
| P45 | run.md:61-67 | Degraded verdicts, human goal-vs-tree read, ht-910 F6 | ROAST | Process items for the final gate. |
| F1 | harness/setup.rs | Allowance printed for all versions | FIXED | 9aafed3 (ancestor of HEAD); README:71 and install.md:13 say 0.159.2-only. |
| F2 | validate-native-demo.py | SIG_IGN inherited by cleanup | FIXED | 77601a0; `default_signals` present. |
| F3 | validate-native-demo.py | Skipped clear phase PASSed | FIXED | 77601a0; `skipped_phases` present. |
| F4 | validate-native-demo.py | S14 read a different CODEX_HOME | FIXED | 266d756; `setup_codex_home`. |
| F5 | cli/hook.rs | Startup context B1/S1/N1-N3 | FIXED | ce530ce. |
| F6 | cli/hook.rs:111-124 | `--state-dir=VALUE` ignored | FIXED | 8fda8d0; `strip_prefix(flag)…'='` branch. |
| F7 | evidence | `Bash(herdr-threads *)` matching unobserved | FIXED | 1246561 (Claude demo 4 PASS). |
| F8 | validate-native-demo.py | auto relaunch on transient event | FIXED | c8b6f1c. |
| F9 | validate-native-demo.py | child-ACK mislabel | FIXED | 77601a0. |
| W5-1 | store/mod.rs:192-216 decision_fence | `known_invalidated` true after reconfirmation (raw observed_targets row) | ROAST | Unchanged. Fails closed (native permit refused until a current-target read); a liveness question that needs a test. |
| W5-2 | store/messages.rs stage_recipient | Behaviour change is unpinned by a test | ROAST | Test gap in a non-safety direction (stages unavailable). |
| W5-3 | service/workers.rs ~1239 | `transitions_refused` never read; `last_reconciliation_at` advances on refusal | ROAST | Only written (identity/reconcile.rs:484,552). Observability. |
| W5-4 | store/seats.rs:1952 Move arm | No test for Guard 2 (registered + occupant absent → no move) or unregistered-after-Move | FIX-NOW | 72e6aec relaxed the Move arm to carry seats without a verified execution. The only remaining guard against carrying a registered binding onto an absent occupant (`registered && observed.shows_occupant_absent()`) has no test. |
| W6-R1 | cli/mod.rs CallerNeed::SelfMarker | Extra daemon connect and full seat page walk on every show/participants | ROAST | Latency on reads only. |
| W6-R2 | harness/mod.rs:242 | "mark it self" wording holds only in-pane | ROAST | Model-facing wording; could add "when run in this pane". |
| W6-R3 | cli/hook.rs | digest None drops the D2 procedure line | ROAST | Continuation still reaches `inbox`. |
| W6-R4 | cli/commands.rs:971 | Participants construction duplicated | WAIVE | Refactor nit. |
| W6-R5 | design docs | harness/cli design not updated for rank, self, alias | ROAST | Docs debt. |
| W6-C1 | harness/launch.rs:120-143 CODEX_VALUE_OPTIONS | Missing `--remote`, `--remote-auth-token-env`, `--thread-source`; `-i` takes several values | FIX-NOW | Still missing. `exec --thread-source X resume ID` is classified as Exec (launch.rs:246-263), so the owned hooks go after `exec`, not after `resume`. The result is a silently hookless managed launch, on the path whose whole purpose is correct placement. |
| W6-C2 | harness/launch.rs:317-331 | Whole-table `-c hooks={...}` / `hooks=[...]` bypasses the owned-hook refusal | FIX-NOW | Check is still `starts_with("hooks.")`. A caller can silently replace the owned hooks on the refusal path meant to prevent exactly that. |
| W6-C3 | harness/launch.rs:264 | Top-level `resume` accepted without live evidence | ROAST | Design choice: refuse, or track a capture. |
| W6-C4 | bead ht-4is.8.6 | Acceptance incomplete (no live managed Codex PASS, rc 5, no check-in confirmation) | ROAST | Partly covered by Codex matrix 2 managed PASS (19b9f9d docs); rc 5 early-exit detection is still open. |
| W6-C5 | — | Reviewer verification note | WAIVE | Informational. |
| W6-D1 | validate-native-demo.py:2110 exec_script_commands | Mixed literal and template `cmd:` loses the child `ack`, so SC2/S18C can false-PASS | FIX-NOW | Unchanged: returns literals only when any exist. A validator false PASS on the child-write safety check. |
| W6-D2 | validate-native-demo.py:126,237 | `service disconnect` missing from MUTATING_VERBS; `"$(which herdr-threads)"` not matched | FIX-NOW | `service disconnect` exists (commands.rs:510,1181) and writes. The lookahead `(?=\s)` fails on `)`. Same false-PASS class; trivial fix. |
| W6-D3 | validate-native-demo.py SM3 | SM2 PASS delivery not counted as "seen" | ROAST | Weakens SM3 to NOT_EXERCISED. |
| W6-D4 | validate-native-demo.py instruction_seen | TUI without transcript turns FAIL into NOT_EXERCISED | ROAST | Should be UNVERIFIED. |
| W6-D5 | reproduction notes | S15 fails with a long run-root (4 KB context plus 23 threads); Claude 2.1.286 out of recipe | ROAST | Claude part fixed by ceff589 (recipe widened to 2.1.286). Run-root and context-budget sensitivity remains; related to P20 and possibly improved by 6ca0b65. |
| W6-D6 | validate-native-demo.sh | Sentence placement in the scenario list | WAIVE | Cosmetic. |
| W6-D7 | — | Verified-OK note | WAIVE | Informational. |
| W8-1 | store/attention.rs invitation_class | No `check_budget()` between walks | WAIVE | Bounded. |
| W8-2 | store/attention.rs | Undocumented newest-pending fallback | WAIVE | Safe (`with_requirements` checks the id). |
| W8-3 | protocol/attention.rs | Long doc line | WAIVE | Cosmetic. |
| W9-1 | store/wake.rs retry ladder | Pre-send refusals climb the 30s→300s ladder, so a wake can be delayed up to 5 min | ROAST | Design; safe. Consider not escalating on recheck refusals. |
| W9-2 | host/native.rs cooperative_wake_ready | No harness match against the seat's recorded harness | ROAST | Harmless generic hint. |
| W9-3 | dispatch PROMPT_MILLIS vs adapter caps | Docs overstate the 2 s prompt ceiling | ROAST | Fails closed; docs accuracy. |
| W9-4 | host/native.rs tests (~1509-1720) | No test for incarnation or epoch changing between the agent.get recheck and agent.prompt, or for an unknown Herdr error code on agent.prompt | FIX-NOW | The branch `!self.same_incarnation(...) \|\| self.epoch() != target.epoch` after the recheck (native.rs ~805) and the `Err(_) => OutcomeUnknown` mapping are untested. These are the TOCTOU guards before typing into a pane. |
| W9-5 | tests/native/recovery r13 | Shell-phase check does not assert an Unsafe attempt row | ROAST | Weak live check; the unit matrix covers it. |
| W9-6 | — | Residual limits documented | WAIVE | Matches design residual-race clause. |
| W9-7 | — | Verified note | WAIVE | Informational. |
| W10-E1 | validate-native-demo.py SL1 | Seq non-null accepted without checking it covers the handoff | ROAST | Validator precision (possible false PASS). |
| W10-E2 | lostprompt semantics | No offered-at-check-in NOT_EXERCISED branch | ROAST | Needs semantic decision with 5.6. |
| W10-E3 | lostprompt_wait | Skips judge, so a usage-limit screen counts as a product FAIL | ROAST | False FAIL direction. |
| W10-E4 | BLOCKEDUI_PROMPT comment (py:89-90) | Stale comment (the path is `/ht-blockedui-*`) | WAIVE | Comment only (verified at py:2787). |
| W10-E5 | TUI Codex user config and hook-trust bypass | Not recorded in evidence or docs | ROAST | Fold into the P4 docs fix where relevant. |
| W10-E6 | rollout_child_call | Spawn count inflated | ROAST | Validator precision. |
| W10-E7 | SK1 (Claude) | One child can count as two readers; background Task looks sequential | ROAST | Validator precision (possible false PASS on the children scenario). |
| W10-E8 | manifest branch | not_exercised branch not limited to lostprompt | ROAST | Reason label precision. |
| W10-E9 | auto + TUI | No profile fallback; undocumented | WAIVE | Result is ENVIRONMENT. |
| W10-E10 | — | Verified-offline note | WAIVE | Informational. |
| W10-Doc1 | README.md:41 | "starting the agent by hand … works the same way" is false for Codex without the `-c` overrides | FIX-NOW | Still verbatim. Merge with the P4 fix. |
| W10-Doc2 | docs/install.md:176 | Claude matrix 1 cited without a version | WAIVE | Style. |
| W10-Doc3 | docs/operations.md:3 | "at 61869bc" claim | WAIVE | Holds (docs-only diff since). |
| W10-Doc4 | — | Verified-OK note | WAIVE | Informational. |
| ht-4is.11.8 | tests/hook_entrypoint.rs | Exits 101 under the parallel harness | WAIVE | CI is serial (ci.yml:102); root waived for this run; not trivially fixable without a root-cause hunt. |

100 entries total (45 Open, 9 Fixed, 4 wave 5, 17 wave 6, 3 wave 8, 7 wave 9, 14 wave 10, plus ht-4is.11.8). FIXED: 10 (P30, F1-F9). OBSOLETE: 0. FIX-NOW: 9 rows (P4, P6, W5-4, W6-C1, W6-C2, W6-D1, W6-D2, W9-4, W10-Doc1), in 7 fix items. ROAST: 40. WAIVE: 41 (including ht-4is.11.8, P12, P13).
## FIX-NOW lane review minors (parked for the roast)
- docs/install.md:119 (fingerprint-cache paragraph): the new sentence about `launch --kind codex` warming the cache was inserted into the middle of a two-item list, so it now reads '...later hooks skip the scan). `launch --kind codex` ... warms it for the hook; and `codex-hook-admission.json`, ...'. The list no longer parses cleanly. It also doesn't say the warm-up is best effort: when the state root is missing or not owned, launch silently falls back to the in-memory cache (src/cli/launch.rs:507-515).
- src/harness/launch.rs:333-355: the `-c`/`--config` value extraction and the `-i`/`--image` refusal run on every option-region argument, including positionals before `--`. A literal prompt argument such as `exec -i` or `exec '-chooks=x'` would be refused. This errs toward refusing, so it is harmless, but the `previous_takes_value` guard covers only option values, not positionals.
- src/host/native.rs cooperative_wake_refuses_when_incarnation_or_epoch_moves_during_recheck: the two server-incarnation cases change the target's recorded incarnation/boot instead of having the recheck witness report a different server. The recheck compares the same values either way, and the author disclosed this. Only the epoch-bump variant actually changes state during the recheck.
- scripts/validate-native-demo.py:243-244: stripping leading `)"'` and backticks after the name, together with the wider lookahead, also matches a quoted literal like `echo "herdr-threads" ack` as a mutating call. That is a false positive, which is the safe direction for a child-no-write check. Noted only because no test documents it.
