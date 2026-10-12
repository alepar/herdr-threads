# Task 9 implementation report

Task: ht-0lo.4.2, owned stable registration migration and tooling.

Source branch: `task-ht-0lo.4.2`; prerequisite/base `217a47e21976c223307ca80e14d53e2e4ee7a991`. Worktree: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.4.2`. Implementation commit: `9f23a2249c0e430ec8107a85feed55fd997f940a` (`Install stable native hooks with exact manifest ownership`). Review fix round 1 commit: `6d4d84492bafb8616efdc8c970d9fc5f557c36ed` (`Refuse conflicting supported legacy native registrations`). The task worktree is clean. Focused checks, mandated clippy, format and scoped process cleanup passed. Fresh scoped rereview and integration are controller-owned.

## Implementation and ownership

Native Claude/Codex setup now consumes the Task3 complete event argv producer and `native_event_command`. New per-event strings have no installation UUID comment; v3 manifests retain installation identity, exact groups and SHA256 fingerprints independently. Equivalent reinstall retains identity, bytes and group positions. Explicit setup migrates exact owned v1/v2 shared/evented registrations through a prepared intent, preserves unrelated groups, replaces superseded groups in their existing positions and recovers interrupted publication. Historical v1 ownership receives a new identity honestly.

Unmanaged identical stable commands remain foreign. Exact duplicate groups alongside a valid manifest, missing or drifted ownership, and ambiguous same-command groups refuse safely. Recognized same-event native registrations on a distinct route also block setup/reinstall; exact unsetup can remove the uniquely proven owned group while retaining a distinguishable foreign route. Recovery uses the exact recorded generation present when a declaration changed metadata while retaining the command, and still validates cardinality. Valid canonical legacy UUID-marked adoption remains available. Ordinary setup/status/unsetup have no explicit adoption flag; fresh copied stable groups carry no canonical legacy ownership evidence, so public copied-profile tests now require refusal and preservation.

Status exposes actual per-event `commands` and diagnostic-only `registration_conflict`. `HookInspection.configured_hook` retains its existing full-current-config fingerprint and structural ownership safety meaning; readiness policy is Task10-owned. The legacy `command`/`shared_command` representation remains the canonical executable/global route, including for v3 event records, so launch/mod consumers keep their existing API. Adopted provenance records the retained installation identity, without claiming that current stable strings contain the old marker. Actual rewriting is reported as a change and emits migration/retrust warnings; legitimate unchanged-byte explicit adoption remains distinct.

Codex trust inspection compares actual event strings. No native trust, hooks.state or permission records are written. Hermes retains its fixed Python bridge. Canary setup assertions and the offline native-demo verifier require exact manifest identity/group/fingerprint/cardinality evidence; stable string equality is never ownership. Capture and Rust replay readers recognize declared stable native event pairs and preserve accepted historical legacy capture forms.

Accepted capture boundary: capture replay retains its historical suffix-oriented recognition contract. The stable suffix validates the declared native harness/event pair; it does not newly validate every preceding argv route word. This is a replay consumer limit, not installation ownership or execution authorization.

## Write surfaces

Admitted changes: `src/harness/setup.rs`, `src/harness/claude/setup.rs`, `src/harness/codex/setup.rs`, `src/cli/setup.rs`, `tests/harness/setup.rs`, `tests/setup_cli.rs`, and Claude/Codex integration READMEs.

Controller-approved additional surfaces, reported before editing: `scripts/harness-canary.sh` (actual capture recognizer and exact manifest setup/unsetup assertions), `scripts/canary/test_tier0_checks.py`, `scripts/canary/test_seam_integration.py`, `tests/harness/canary_payloads.rs` (actual replay reader), `src/cli/doctor.rs` (truthful legacy migration hint only), `scripts/validate-native-demo.py`, and its existing isolated offline test `tests/native/demo/test_demo_verify.py`. No new test target, manifest writer implementation or launcher/Hermes/mod production source was added.

## Rust TDD and verification evidence

Every Cargo invocation used the existing absolute worktree `target`, `CARGO_BUILD_JOBS=4`, `nice`, `--locked --all-features`, controller-exclusive lane, and scoped CLI escalation. Source/scripts were frozen while Cargo ran. No cold baseline, alternate/shared target, profile change, timeout increase or budget relaxation was used. Timings below are actual command observations, not estimates.

Common command environment for the final package:

```sh
HT_LEAK_RUN_ID=a6e9c882-fd2d-4fcb-8f1f-a4d924209939
CARGO_BUILD_JOBS=4
CARGO_TARGET_DIR=/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.4.2/target
```

Commands below were timed using `/usr/bin/time -p` unless otherwise noted.

| Command | Actual outcome | Build / test / elapsed |
| --- | --- | --- |
| `nice cargo test --locked --all-features stable_` | Fixture compilation error: unavailable tempfile dependency; not RED. Corrected fixtures to the existing UUID tempdir pattern. | Original elapsed not recorded. |
| `nice cargo test --locked --all-features --lib stable_` | First meaningful RED: 16 selected, 12 passed, 4 failed (stable owned repeat, unmanaged identical, UUID/shared migration still legacy, stable replay rejected). No Rust production preceded this RED. | Build 3m39; test 0.36s. Incremental build budget exceeded; active rustc compilation was observed, not a lock wait/hang. |
| `nice cargo test --locked --all-features --lib stable_` | New fixture enum mismatch compile error; not RED. Corrected the fixture harness type before retry. | 10.55s. |
| `nice cargo test --locked --all-features --lib stable_` | Mixed RED: 22 selected, 16 passed, 6 failed at newly gated observer, foreign-route, quoted-marker, v1 recovery and truthful migration-result branches, before changing those branches. | Build 1m07; test 4.99s; real 73.92, user 157.46, sys 4.12. Build budget exceeded. |
| `nice cargo test --locked --all-features --lib stable_` | GREEN: 22 passed. | Build 40.33s; test 1.52s; real 42.87, user 125.21, sys 2.87. |
| `nice cargo test --locked --all-features --lib stable_same_event_foreign_route_conflicts_without_config_changes` | Meaningful RED for a distinct native route beside a valid owned manifest: setup incorrectly returned success. Added at the controller's named acceptance gap before installed-path fix. | Build 10.18s; test 0.07s; real 11.21, user 13.77, sys 1.35. |
| Same narrow command | Meaningful status RED after setup refusal fix: ownership remained installed as required, but `registration_conflict` was absent/null instead of true. No status production change preceded this RED. | Build 1m05; test 0.11s; real 66.46, user 84.38, sys 3.31. Build budget exceeded. |
| `nice cargo test --locked --all-features --lib stable_` | GREEN: 22 passed after bounded route and diagnostic status fixes. | Build 30.45s; test 1.94s; real 33.38, user 84.14, sys 2.56. |
| `nice cargo test --locked --all-features --lib harness::setup_tests` | 83 selected: 77 passed, six existing declaration-drift/recovery tests failed. These exposed same-command metadata generations incorrectly treated as foreign. Lane released before the bounded fix. | Build 39.68s; test 3.98s; real 44.62, user 69.40, sys 3.61. |
| Same setup command, after exact-generation fix | GREEN: 83 passed, 0 failed; all six prior failures and the full stable ownership matrix passed without weakening assertions. Session 42026 exit 0; lane released immediately. | Build 40.90s; test 4.73s; real 46.57, user 71.93, sys 3.79. |
| `nice cargo test --locked --all-features --lib harness::launch::tests` | GREEN: 35 passed. | Build 0.05s; test 0.00s; real 0.13, user 0.07, sys 0.04. |
| `nice cargo test --locked --all-features --lib harness::claude_mod::tests` | GREEN: 25 passed, including stable/legacy launch-prefix compatibility. | Build 0.04s; test 2.44s; real 2.53, user 0.06, sys 0.41. |
| `nice cargo test --locked --all-features --lib harness::canary_payloads` | GREEN: 8 passed. | Build 0.05s; test 0.01s; real 0.11, user 0.05, sys 0.04. |
| `nice cargo test --locked --all-features --test combined setup_cli` | GREEN: 82 passed, including actual CLI native migration, copied stable foreign groups, doctor, trust nonmutation, Hermes and Claude mod. | Build 59.63s; test 9.24s; real 69.57, user 130.01, sys 8.74. Build under one minute. |
| `nice cargo test --locked --all-features --test combined canary_manifest_writer` | GREEN: 3 passed. | Build 0.06s; test 0.97s; real 1.12, user 0.25, sys 0.13. |
| `nice cargo test --locked --all-features --test integration setup_canary_seam` | GREEN: 5 passed against actual isolated built setup and shell consumers. | Build 18.26s; test 4.84s; real 23.78, user 35.02, sys 2.54. |
| `nice cargo clippy --locked --all-targets --all-features -- -D warnings` | PASS, exit 0, all targets and all features. | Build/check 42.84s; real 43.00, user 68.77, sys 1.94; under one minute. |
| `cargo fmt --check` | PASS, exit 0. | Real 3.87, user 3.77, sys 0.08. |

The six broad failures were `changed_file_drift_upgrade_keeps_empty_owned_event_arrays_the_recorded_base_held`, `changed_file_drift_upgrade_removal_leaves_no_emptied_owned_event_array`, `interrupted_drift_upgrade_resumes_or_removes_from_each_real_boundary`, `prepared_install_under_earlier_declaration_converges_on_resetup`, `prepared_resetup_refuses_second_supersession_of_the_same_event`, and `resetup_after_declaration_drift_upgrades_to_the_current_declaration`.

The new matrix covers both native harnesses, v1/v2/v3 ownership, shared/evented UUID migration, intent/publication interruptions and recovery/removal, stable identity/order/byte preservation, foreign identical groups with and without valid manifests, exact duplicate ambiguity before/after owned groups, drifted groups, distinct routes, read-only status, quoted executable/path marker text, observer recognition and actual migration-result/retrust provenance.

## Offline Python TDD evidence

Capture recognition first failed on the real stable captured argv with “no capture whose argv ends hook claude”; the stable recognition fix passed the focused/full tier0 checks. Exact stable setup ownership first failed with “no owned SessionStart”; unsetup residue first incorrectly passed a leftover stable command. A same-command drifted duplicate also first incorrectly passed exact-manifest setup assertion. Each behavior changed only after its meaningful offline RED. Final `python3 scripts/canary/test_tier0_checks.py`: 17 passed in 16.141s. Earlier successive runs were 15 passed/23.739s and 17 passed/28.483s; focused fixes passed in 1.185s, 0.435s and 1.030s respectively.

`python3 scripts/canary/test_manifest.py`: 59 passed in 26.959s (offline fake Cargo/release stubs; production writer unchanged).

Native-demo RED used the existing offline path, `python3 -m unittest discover -s tests/native/demo -p test_demo_verify.py -k test_codex_cli_setup_writes_scratch_hooks_and_no_hook_arguments`: 1 failed in 0.006s because marker-only recognition reported no owned hooks. After the strict verifier and real v2-positive/duplicate-drift fixture updates, `-k SetupModeTests`: 7 passed in 0.103s; `-k CodexTransportSetupTests`: 6 passed in 0.090s. No live demo or native harness was invoked.

`bash -n scripts/harness-canary.sh` and Python byte-compilation of the changed Python files passed.

`nice python3 scripts/canary/test_seam_integration.py SeamIntegration.test_expected_admission_matches_versions_py_and_the_gated_test_honours_it`: PASS, 1 test in 3.751s; real 3.81, user 1.83, sys 0.76. This actually invoked `nice cargo test --locked --all-features --lib gated_canary_payloads` under the same exclusive lane, persistent target, jobs4 and scoped leak environment. It retained actual declared admission/event validation and did not treat a diagnostic version as qualification.

## Self-review, cleanup and remaining controller work

Self-review inspected exact ownership/cardinality, valid manifest plus unmanaged duplicate before/after the owned group, canonical quoted marker boundaries, historical v1 identity, migration result/adoption provenance, declaration-drift recovery, status/HookInspection separation and actual shell/demo consumer evidence. No further source change was needed after the final setup83 fix. `git diff --check` passed. No source or script changed during final verification; final commit contains the verified source.

`scripts/check-no-leaked-processes --run-id a6e9c882-fd2d-4fcb-8f1f-a4d924209939`: exit 0, `no leaked test processes`. All owned command sessions completed. Removed the ten specifically identified failed `ht-task9-<UUID>` file-only fixture directories and the uniquely identified quoted-marker fixture `herdr-setup-601b6349-fc9c-49c6-9f84-0c2cf456593a`. Generic failed older setup fixture artifacts were not glob-deleted because another worktree may use that prefix; they contain files only, not running children. No foreign process was signaled.

The first sandboxed `git add` could not create the worktree metadata index lock; the authorized explicit-path staging and commit then succeeded with scoped escalation. No automatic approval review rejected an action.

Parent owns fresh review, any later dependency composition/rebase, default-feature check, final readiness and integration/main merge. This commit does not touch `integrations/claude-mod/register.js`; later composition must preserve the parent's newer unprefixed `io.log` fix. No local full-suite/default-feature readiness claim is made. Measured earlier build budget overruns remain reported above; no budget was raised to hide them.

No real `~/.claude`, `~/.codex` or `~/.aisw` config, native trust record, shared Herdr process or foreign worktree was modified. Fixtures use isolated temp configs and owned/tagged children. No push, stash, live native demo or herdr-threads command was run.

## Fresh-review fix round 1 — Important I1

Review `task-9-review.md` identified a real compatibility hole: the conflict predicate only recognized stable dedicated commands, so canonical supported shared/evented legacy commands on a foreign route could execute alongside newly installed stable hooks and status falsely reported no registration conflict. The review's actual isolated public CLI proof was consistent with the source validator. This is fixed in the narrow range `FIXBASE 9f23a2249c0e430ec8107a85feed55fd997f940a..6d4d84492bafb8616efdc8c970d9fc5f557c36ed`.

Only two already admitted files changed in this round: `src/harness/setup.rs` (30 lines) and `tests/setup_cli.rs` (117 lines). The bounded recognizer accepts canonical `installed_argv` legacy shared commands or exact `event_command` legacy evented commands for the current event, optionally followed by a real unquoted trailing canonical UUID owner marker. It rejects arbitrary shell text or noncanonical reconstruction. The shared setup/status conflict predicate now checks this alongside the stable form. Recognition confers no ownership; exact recorded groups are still the only migration/recovery exceptions. HookInspection, trust, migration/adoption, recovery, unsetup ownership and external APIs were not changed.

Before production changes, eight independent actual public CLI regressions were added: Claude/Codex × shared/evented legacy UUID commands × fresh/already owned installation. Each seeds a distinct foreign route alongside ordinary hooks before and after, checks public ownership status, setup refusal with the existing actionable diagnostic, truthful `registration_conflict`, config and manifest byte preservation, and exact unsetup retaining the distinguishable foreign and ordinary groups.

All commands below used the same existing absolute target/jobs4/nice/scoped escalation and leak-run environment documented above. Source was frozen during every command. The eight narrow tests overlap the 90-test CLI selection: the fix round verifies **173 unique Rust tests**, not 181.

| Exact command | Actual result | Timing |
| --- | --- | --- |
| `/usr/bin/time -p nice cargo test --locked --all-features --test combined legacy_foreign_registration_` before production fix | Meaningful RED: 0 passed, 8 failed; every case returned successful `installed` or `already_installed` instead of refusal. Session 58286 exit 101; lane released before changes. | Build 4.48s; test 1.85s; real 7.05, user 5.07, sys 0.96. |
| Same narrow command after bounded fix | GREEN: 8 passed. Session 47815 exit 0. Public refusal, status, nonmutation and distinguishable unsetup assertions passed for every matrix case. | Build 38.17s; test 1.76s; real 40.63, user 47.48, sys 2.81. |
| `/usr/bin/time -p nice cargo test --locked --all-features --lib harness::setup_tests` | GREEN: 83 passed, including valid legacy adoption, v1/v2/v3 owned migration/interruption/recovery, duplicate ambiguity and declaration drift. Session 99142 exit 0. | Build 47.24s; test 6.85s; real 55.10, user 79.22, sys 5.30. |
| `/usr/bin/time -p nice cargo test --locked --all-features --test combined setup_cli` | GREEN: 90 passed, including the eight new cases and existing native trust, doctor, Hermes/mod and adoption behavior. Session 20341 exit 0. | Build 0.22s; test 14.06s; real 14.32, user 7.79, sys 7.53. |
| `/usr/bin/time -p nice cargo clippy --locked --all-targets --all-features -- -D warnings` | PASS, session 13342 exit 0. | Check 25.08s; real 25.19, user 31.74, sys 2.58. |
| `/usr/bin/time -p cargo fmt --check` | PASS, exit 0. | Real 4.93, user 4.15, sys 0.22. |
| `scripts/check-no-leaked-processes --run-id a6e9c882-fd2d-4fcb-8f1f-a4d924209939` | PASS, exit 0: `no leaked test processes`. All sessions completed; Cargo lane released before commit/report. | No separate runtime recorded. |

Formatting preparation initially invoked direct rustfmt with edition 2021, which refused existing edition-2024 let chains; this was not a Cargo or test RED. The declared Cargo edition was verified and direct `rustfmt --edition 2024 src/harness/setup.rs tests/setup_cli.rs` passed, followed by the mandated Cargo format check above. No fixture assertion was weakened and no new write surface, cache, budget, profile or timeout was introduced.

Final narrow self-review confirmed canonical legacy parsing remains separate from ownership, marker stripping uses the existing shell-boundary/UUID validator, wrong event strings do not match, and exact manifest exception logic is unchanged. `git diff --check` passed. Scratch fixtures remove themselves on drop, including failed RED cases; the scoped process scan was clean. The fix commit contains exactly the tested two-file change, and `git status --porcelain=v1` is empty. Ready for controller fresh scoped rereview; no full-suite/default-feature or later integration readiness is claimed.
